//! Image loading abstraction for rinch.
//!
//! Defines the [`ImageLoader`] trait which backends use to load images
//! from various sources (local files, network, etc.), and the registry of
//! **app-installed URL schemes** ([`register_image_scheme`]) that lets an app
//! answer for sources only it can resolve (`myapp-blob:…`) while every other
//! source keeps going to the document's own loader.
//!
//! A picture the app holds as **decoded pixels** (a notification's raw image,
//! a frame it made, a file it decoded itself) needs no loader at all:
//! [`register_rgba`] puts it under a name, and `<img src="rinch-image:name">`
//! ([`rgba_src`]) shows it. A loader can answer with pixels too
//! ([`ImageLoadResult::Decoded`]).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

/// Result of an image load operation.
pub enum ImageLoadResult {
    /// Successfully loaded raw **encoded** bytes: the contents of a PNG, JPEG,
    /// GIF or WebP file, exactly as they would sit on disk. rinch decodes them
    /// (on the thread that called the loader). Pixels the app holds already
    /// decoded are [`Decoded`](Self::Decoded).
    Loaded(Vec<u8>),
    /// Successfully loaded **decoded** pixels: `width` x `height` RGBA8, four
    /// bytes a pixel, rows top to bottom with no padding between them, and
    /// **straight** (not premultiplied) alpha, the form PNG and the
    /// freedesktop notification `image-data` hint use. Premultiplied pixels
    /// are not accepted: hand over straight ones (rinch premultiplies
    /// internally where a painter needs it).
    ///
    /// rinch does not decode them. It checks them ([`check_rgba`]): a buffer
    /// whose length is not `width * height * 4`, an empty image, or one past
    /// [`MAX_DECODED_IMAGE_BYTES`] (the limit an encoded file's decode has)
    /// fails the load, as [`Failed`](Self::Failed) would, and never panics.
    /// From then on it is any decoded picture: cached by source, drawn by both
    /// painters, fitted by `object-fit`, reloaded by `reload_image`.
    ///
    /// The pixels are shared, not copied: the GPU painter hands the same
    /// buffer to Vello, and the software painter keeps them as they are when
    /// every pixel is opaque (otherwise it makes one premultiplied copy, once
    /// per image). So an app that keeps the same pixels in its own model can
    /// answer `rgba.clone()` of its `Arc` at the cost of a reference count.
    ///
    /// This is also the way to show an **untrusted file under the app's own
    /// bounds**: decode it yourself, with whatever limits on size and format
    /// you want, and answer with the pixels.
    Decoded {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// `width * height * 4` bytes of straight-alpha RGBA8.
        rgba: Arc<[u8]>,
    },
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
/// The **encoded** bytes of the image ([`ImageLoadResult::Loaded`]), which
/// rinch decodes; or its **decoded** pixels ([`ImageLoadResult::Decoded`],
/// straight-alpha RGBA8), which it only checks; or a failure.
///
/// # Where it runs
///
/// On native targets, on a background thread spawned for that one load
/// ([`std::thread::spawn`]), never on the UI thread, so blocking I/O is
/// acceptable: a loader may read a
/// file, make a request, or wait on a channel. It is called once per source
/// per document (results are cached by source string), and from several
/// threads at once when several sources are loading, hence `Send + Sync`.
///
/// A loader that blocks for a long time holds its thread for that long. One
/// that does not know yet whether the bytes will ever exist should answer
/// [`ImageLoadResult::Failed`] and have the app call `reload_image` when they
/// arrive, rather than park a thread per missing picture.
///
/// On `wasm32` (an embedded `RinchContext` in the browser, a WASI guest) there
/// are no threads to spawn: the loader is called on the UI thread, by the
/// frame (`RinchContext::update`) that first names the source, with no
/// document borrowed, so it may write signals; its bytes are decoded there.
/// It must answer from memory, never wait; a source it has to fetch first is
/// answered [`ImageLoadResult::Failed`] and reloaded when the bytes arrive.
///
/// On desktop (any native target built with unwinding), a loader that
/// **panics** has failed that load: the panic is caught on the load's thread
/// and recorded as an [`ImageLoadResult::Failed`] carrying its message, so the
/// source can be reloaded like any other failure. In the browser, and on a
/// native build with `panic = "abort"`, a panic aborts the whole app. A loader
/// must not call `reload_image` for the source it is being asked for: that
/// discards the answer it is about to give and asks again, for ever, and keeps
/// a reload of it queued every frame, so a host that idles when nothing is
/// pending never idles.
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
/// takes it over. `data` is the one scheme that cannot be registered: no
/// loader is ever offered a `data:` URL (an `<img>`'s and a
/// `background-image`'s are decoded in place).
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

/// The app-installed loader that answers for `src`, if its scheme has one,
/// else the built-in one for [`RGBA_SCHEME`] (the [`register_rgba`] registry)
/// when that is `src`'s scheme.
pub fn image_scheme_loader(src: &str) -> Option<Arc<dyn ImageLoader>> {
    let scheme = scheme_of(src)?;
    let app = SCHEME_LOADERS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(s, _)| *s == scheme)
        .map(|(_, loader)| loader.clone());
    app.or_else(|| (scheme == RGBA_SCHEME).then(|| RGBA_LOADER.clone()))
}

/// The most bytes a decoded picture may take, 512 MiB: `width * height * 4`
/// of [`ImageLoadResult::Decoded`] and [`register_rgba`] must not pass it.
///
/// It is the limit rinch's decode of an **encoded** file has (the `image`
/// crate's default allocation limit, which `rinch-dom` passes to its decoder
/// from this constant), so pixels handed over decoded can be no larger than a
/// file rinch would have decoded. An app showing untrusted pictures will want
/// a far smaller bound of its own: decode and check them itself, then hand
/// over the pixels.
pub const MAX_DECODED_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// The built-in URL scheme [`register_rgba`]'s pictures are shown under:
/// `rinch-image:<name>` ([`rgba_src`]).
///
/// An app that registers a loader for this scheme itself
/// ([`register_image_scheme`]) takes it over, and the registry is then not
/// asked.
pub const RGBA_SCHEME: &str = "rinch-image";

/// Why decoded pixels were refused ([`check_rgba`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedImageError {
    /// The width or the height is 0.
    Empty {
        /// The width given.
        width: u32,
        /// The height given.
        height: u32,
    },
    /// `width * height * 4` is past [`MAX_DECODED_IMAGE_BYTES`].
    TooLarge {
        /// The width given.
        width: u32,
        /// The height given.
        height: u32,
    },
    /// The buffer is not `width * height * 4` bytes long.
    WrongLength {
        /// `width * height * 4`.
        expected: u64,
        /// The buffer's length.
        actual: usize,
    },
}

impl std::fmt::Display for DecodedImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty { width, height } => write!(f, "a {width}x{height} image has no pixels"),
            Self::TooLarge { width, height } => write!(
                f,
                "a {width}x{height} RGBA image is past the limit of \
                 {MAX_DECODED_IMAGE_BYTES} bytes"
            ),
            Self::WrongLength { expected, actual } => write!(
                f,
                "RGBA pixels are {actual} bytes, not the {expected} that width x height x 4 is"
            ),
        }
    }
}

impl std::error::Error for DecodedImageError {}

/// Check decoded pixels as rinch does before it takes them: `width` and
/// `height` not 0, `width * height * 4` within [`MAX_DECODED_IMAGE_BYTES`],
/// and the buffer (`len` bytes) exactly that long. The size is checked first,
/// so an oversized claim is refused for what it claims, whatever the buffer.
pub fn check_rgba(width: u32, height: u32, len: usize) -> Result<(), DecodedImageError> {
    if width == 0 || height == 0 {
        return Err(DecodedImageError::Empty { width, height });
    }
    // `u32::MAX` squared times 4 does not fit a `u64`: saturate, which is
    // past the limit either way.
    let expected = (u64::from(width) * u64::from(height)).saturating_mul(4);
    if expected > MAX_DECODED_IMAGE_BYTES {
        return Err(DecodedImageError::TooLarge { width, height });
    }
    if len as u64 != expected {
        return Err(DecodedImageError::WrongLength {
            expected,
            actual: len,
        });
    }
    Ok(())
}

/// One [`register_rgba`] picture.
struct RgbaEntry {
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

/// The [`register_rgba`] pictures by name, process-wide (loads run on
/// background threads and every document shares them, as scheme loaders are).
static RGBA_IMAGES: LazyLock<RwLock<HashMap<String, RgbaEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The loader of [`RGBA_SCHEME`]: answers from [`RGBA_IMAGES`].
static RGBA_LOADER: LazyLock<Arc<dyn ImageLoader>> = LazyLock::new(|| {
    Arc::new(|src: &str| match rgba_name(src) {
        Some(name) => match RGBA_IMAGES
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
        {
            Some(entry) => ImageLoadResult::Decoded {
                width: entry.width,
                height: entry.height,
                rgba: entry.rgba.clone(),
            },
            None => ImageLoadResult::Failed(format!("no image is registered as `{name}`")),
        },
        None => ImageLoadResult::Failed(format!("`{src}` is not a {RGBA_SCHEME}: source")),
    })
});

/// What the built-in [`register_rgba`] registry answers for `src` now, if it
/// is the one to answer: `src` is a [`RGBA_SCHEME`] source and no app loader
/// has taken the scheme over. `None` otherwise, and the source is loaded as
/// any other (on a background thread, through whichever loader answers it).
///
/// The registry's pixels are decoded and in memory already, so a rendering
/// backend resolves such a source in the frame that names it (#2857): no
/// thread hop, so an element whose picture moves from one registered name to
/// another draws the new one in that frame, and an unknown or unregistered
/// name fails in it ([`ImageLoadResult::Failed`]). For rendering backends.
///
/// With the answer comes the loader that gave it (the built-in one, the
/// same `Arc` [`image_scheme_loader`] answers for the scheme), read in the
/// same look at the app's loaders, so that a backend keying a cache by
/// loader cannot key the registry's answer by a loader an app installed in
/// between.
#[doc(hidden)]
pub fn registered_rgba_answer(src: &str) -> Option<(Arc<dyn ImageLoader>, ImageLoadResult)> {
    let scheme = scheme_of(src)?;
    if scheme != RGBA_SCHEME {
        return None;
    }
    let taken_over = SCHEME_LOADERS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(s, _)| *s == scheme);
    (!taken_over).then(|| (RGBA_LOADER.clone(), RGBA_LOADER.load(src)))
}

/// Whether `src` is a [`RGBA_SCHEME`] source, whoever answers it: what a
/// backend checks before recording that a document asks for it.
#[doc(hidden)]
pub fn is_rgba_source(src: &str) -> bool {
    rgba_name(src).is_some()
}

/// The name in a `rinch-image:<name>` source (scheme in any case), or `None`
/// for a source of another scheme.
fn rgba_name(src: &str) -> Option<&str> {
    let (scheme, name) = src.split_once(':')?;
    scheme.eq_ignore_ascii_case(RGBA_SCHEME).then_some(name)
}

/// The source that shows the picture [`register_rgba`] put under `name`:
/// `rinch-image:<name>`, for `<img src>` (or a `background-image: url(…)`).
/// The name is kept as given, case included.
pub fn rgba_src(name: &str) -> String {
    format!("{RGBA_SCHEME}:{name}")
}

/// Show decoded pixels under `name`, without writing a loader: every
/// `<img src="rinch-image:<name>">` ([`rgba_src`], which this returns) in
/// every window shows them.
///
/// `rgba` is `width * height * 4` bytes of **straight-alpha** RGBA8, rows top
/// to bottom with no padding (see [`ImageLoadResult::Decoded`]). It is
/// checked here ([`check_rgba`]): a wrong length, an empty image or one past
/// [`MAX_DECODED_IMAGE_BYTES`] is refused with the reason, and nothing changes.
/// The pixels are kept shared, not copied: pass an `Arc<[u8]>` the app already
/// holds and the registry, the cache and the GPU painter all read that one
/// buffer.
///
/// Registering a name **again** replaces its picture: every element showing
/// it is repainted with the new one (and re-laid out if its size changed),
/// the old one staying on screen until the new one is in. A name registered
/// after an element asked for it (and drew the broken-image state) is shown
/// too. [`unregister_rgba`] takes it away.
///
/// An element **given** a registered name (a new `src`, a new
/// `background-image`) draws its picture in the frame that applies the
/// change: the registry answers at once, not from a load thread
/// ([`registered_rgba_answer`]). An app loader for [`RGBA_SCHEME`] takes this
/// over, and is asked on the load thread like any loader.
///
/// Callable from **any thread** (a notification daemon's D-Bus thread, a
/// decoder's worker): the registry is behind a lock, and the repaint is
/// queued for each document's next frame and wakes the UI thread
/// ([`reload_image`]). In the browser backend (`rinch-web`) the pixels are
/// drawn into a canvas once and its PNG `data:` URL given to the `<img>`; the
/// canvas stores premultiplied pixels, so a colour under a very small alpha
/// may come back rounded there.
pub fn register_rgba(
    name: &str,
    width: u32,
    height: u32,
    rgba: impl Into<Arc<[u8]>>,
) -> Result<String, DecodedImageError> {
    let rgba = rgba.into();
    check_rgba(width, height, rgba.len())?;
    RGBA_IMAGES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            name.to_string(),
            RgbaEntry {
                width,
                height,
                rgba,
            },
        );
    let src = rgba_src(name);
    reload_image(&src);
    Ok(src)
}

/// Take away the picture [`register_rgba`] put under `name`. Returns whether
/// there was one.
///
/// Every element still showing it **draws no picture** from the next frame:
/// an `<img>` shows the broken-image state, exactly as one whose source failed
/// to load (its `alt` text, or nothing), which is what Chrome shows for an
/// `<img>` whose blob URL has been revoked when it loads; a background draws
/// nothing. The stale picture is not kept, unlike a [`reload_image`] whose
/// loader fails. Callable from any thread.
pub fn unregister_rgba(name: &str) -> bool {
    let removed = RGBA_IMAGES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(name)
        .is_some();
    if removed {
        reload_image(&rgba_src(name));
    }
    removed
}

/// Whether `src` is a [`RGBA_SCHEME`] source the built-in registry answers
/// (no app loader took the scheme over) and nothing is registered under its
/// name now: a failure for it means the picture was taken away, so a backend
/// drops what it showed instead of keeping it. For rendering backends.
#[doc(hidden)]
pub fn is_unregistered_rgba(src: &str) -> bool {
    let Some(name) = rgba_name(src) else {
        return false;
    };
    let taken_over = SCHEME_LOADERS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(s, _)| s == RGBA_SCHEME);
    !taken_over
        && !RGBA_IMAGES
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(name)
}

/// What each rendering backend does when [`reload_image`] is called: the
/// desktop document queues the source for its image cache, the browser backend
/// asks the app's resolver again and re-points its `<img>` elements.
static RELOADERS: RwLock<Vec<fn(&str)>> = RwLock::new(Vec::new());

/// Ask for `src` to be loaded again, wherever it is shown: the way a picture
/// that was not there when it was first asked for (the loader answered
/// [`ImageLoadResult::Failed`]), or whose bytes have changed, gets another go
/// without a restart.
///
/// A load's answer is remembered by source string, **a failure included**, so
/// an app whose loader can answer late (a file still downloading, a blob that
/// has not been synced yet) calls this when the bytes exist. Every element
/// naming `src`, in every window or mounted root, then asks again and shows the
/// picture. A picture already on screen stays there until the new answer
/// replaces it. `data:` sources never change and are ignored.
///
/// Callable from any thread on desktop (it queues the request and wakes the UI
/// thread); in the browser there is one thread and it acts at once. The
/// backend's own notes say what "ask again" means there:
/// `rinch_dom::image_cache::reload_image` and
/// `rinch_web::register_image_url_scheme`.
pub fn reload_image(src: &str) {
    if src.is_empty() || src.starts_with("data:") {
        return;
    }
    let reloaders: Vec<fn(&str)> = RELOADERS.read().unwrap_or_else(|e| e.into_inner()).clone();
    for reload in reloaders {
        reload(src);
    }
}

/// Install a backend's half of [`reload_image`]. For rendering backends
/// (`rinch-dom`, `rinch-web`), which call it once; not for apps.
#[doc(hidden)]
pub fn add_image_reloader(reload: fn(&str)) {
    RELOADERS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(reload);
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
            _ => None,
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

    #[test]
    fn check_rgba_refuses_a_wrong_length_an_empty_image_and_one_past_the_limit() {
        assert_eq!(check_rgba(3, 2, 24), Ok(()));
        assert_eq!(
            check_rgba(3, 2, 23),
            Err(DecodedImageError::WrongLength {
                expected: 24,
                actual: 23
            })
        );
        assert_eq!(
            check_rgba(0, 2, 0),
            Err(DecodedImageError::Empty {
                width: 0,
                height: 2
            })
        );
        // 8192 x 16384 x 4 is exactly 512 MiB: allowed. One row more is not,
        // whatever the buffer holds (it is checked before the length).
        assert_eq!(
            check_rgba(8192, 16384, 0),
            Err(DecodedImageError::WrongLength {
                expected: MAX_DECODED_IMAGE_BYTES,
                actual: 0
            })
        );
        assert_eq!(
            check_rgba(8192, 16385, 0),
            Err(DecodedImageError::TooLarge {
                width: 8192,
                height: 16385
            })
        );
        assert_eq!(
            check_rgba(u32::MAX, u32::MAX, 0),
            Err(DecodedImageError::TooLarge {
                width: u32::MAX,
                height: u32::MAX
            }),
            "no overflow"
        );
    }

    /// The tests that read or take over the built-in `rinch-image` scheme,
    /// which is process-wide, run one at a time.
    static RGBA_SCHEME_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn an_app_loader_for_the_rgba_scheme_takes_it_over() {
        let _alone = RGBA_SCHEME_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let src = rgba_src("core-test-taken");
        assert!(is_unregistered_rgba(&src), "control: the registry answers");
        register_image_scheme("RINCH-IMAGE", |_: &str| {
            ImageLoadResult::Failed("the app's".into())
        });
        assert!(
            !is_unregistered_rgba(&src),
            "the app's loader answers now: its failure is an ordinary one"
        );
        match image_scheme_loader(&src).unwrap().load(&src) {
            ImageLoadResult::Failed(e) => assert_eq!(e, "the app's"),
            _ => panic!("expected the app's loader"),
        }
        assert!(unregister_image_scheme("rinch-image"));
        assert!(is_unregistered_rgba(&src), "the registry answers again");
    }

    #[test]
    fn a_registered_rgba_picture_is_answered_by_the_built_in_scheme() {
        let _alone = RGBA_SCHEME_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let pixels: Arc<[u8]> = Arc::from(vec![1u8, 2, 3, 4, 5, 6, 7, 8]);
        let src = register_rgba("core-test-pic", 2, 1, pixels.clone()).expect("valid");
        assert_eq!(src, "rinch-image:core-test-pic");
        assert!(!is_unregistered_rgba(&src));
        match image_scheme_loader("RINCH-IMAGE:core-test-pic")
            .expect("built in")
            .load("RINCH-IMAGE:core-test-pic")
        {
            ImageLoadResult::Decoded {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (2, 1));
                assert!(Arc::ptr_eq(&rgba, &pixels), "shared, not copied");
            }
            _ => panic!("expected the pixels"),
        }
        assert_eq!(
            register_rgba("core-test-pic", 2, 2, pixels.clone()),
            Err(DecodedImageError::WrongLength {
                expected: 16,
                actual: 8
            }),
        );
        assert!(
            matches!(
                image_scheme_loader(&src).unwrap().load(&src),
                ImageLoadResult::Decoded {
                    width: 2,
                    height: 1,
                    ..
                }
            ),
            "a refused registration changes nothing"
        );
        assert!(unregister_rgba("core-test-pic"));
        assert!(!unregister_rgba("core-test-pic"));
        assert!(is_unregistered_rgba(&src));
        assert!(matches!(
            image_scheme_loader(&src).unwrap().load(&src),
            ImageLoadResult::Failed(_)
        ));
        assert!(!is_unregistered_rgba("other:core-test-pic"));
    }
}
