//! Image cache and loading pipeline for rinch-dom.
//!
//! Manages decoded images for `<img>` elements and `background-image` CSS.
//! Images are loaded asynchronously on background threads and decoded into
//! RGBA8 pixel data suitable for Vello rendering.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use rinch_core::image::{ImageLoadResult, ImageLoader};

/// A decoded image ready to paint.
///
/// A raster image (PNG, JPEG, GIF, WebP) is its pixels. An SVG file (#2111)
/// is a [`VectorImage`](crate::svg_image::VectorImage) ([`Self::vector`]),
/// rasterised at each size it is drawn at; its `data` is empty and `width` x
/// `height` is its [`concrete`](crate::svg_image::VectorImage::concrete) size
/// rounded, so code that reads the pixels directly draws nothing.
pub struct DecodedImage {
    /// Raw RGBA8 pixel data, straight (not premultiplied) alpha. Empty for
    /// an SVG file. Shared: pixels an app handed over decoded
    /// ([`ImageLoadResult::Decoded`]) are its own buffer, not a copy.
    pub data: ImagePixels,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// The parsed SVG file, for an SVG source.
    vector: Option<Arc<crate::svg_image::VectorImage>>,
    /// The same pixels premultiplied, made on the first software paint and
    /// kept (the software painter needs premultiplied pixels, and making them
    /// is a pass over the whole image). `None` inside means every pixel is
    /// opaque, so the premultiplied pixels *are* `data` and no copy is kept.
    /// Never filled on the GPU path, which takes straight alpha.
    premultiplied: std::sync::OnceLock<Option<Vec<u8>>>,
    /// The pixels as the one `peniko::Blob` every GPU paint of this image
    /// hands Vello, made on the first. Vello keys its image atlas on the
    /// blob's id: a blob made per paint was a new image every repaint, which
    /// Vello 0.9+ keeps resident beside the earlier copies, and which was
    /// uploaded again every frame (#1955). Never filled on the software path.
    blob: std::sync::OnceLock<peniko::Blob<u8>>,
    /// Process-unique, never reused: what a cache of pixels derived from this
    /// image keys on (a data pointer can be reused by the next image).
    id: u64,
}

/// A [`DecodedImage`]'s pixels: one shared buffer, read as `&[u8]`
/// (`Deref`). Made from a `Vec<u8>` (rinch's own decodes, no copy) or from an
/// `Arc<[u8]>` an app handed over (no copy either), and given to Vello as the
/// same buffer ([`DecodedImage::blob`]).
#[derive(Clone)]
pub struct ImagePixels(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl std::ops::Deref for ImagePixels {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

impl AsRef<[u8]> for ImagePixels {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl std::fmt::Debug for ImagePixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImagePixels({} bytes)", self.len())
    }
}

impl PartialEq for ImagePixels {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for ImagePixels {}

impl From<Vec<u8>> for ImagePixels {
    fn from(data: Vec<u8>) -> Self {
        Self(Arc::new(data))
    }
}

impl From<Arc<[u8]>> for ImagePixels {
    fn from(data: Arc<[u8]>) -> Self {
        Self(Arc::new(data))
    }
}

impl DecodedImage {
    /// A decoded image over straight-alpha RGBA8 `data`.
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        Self::from_pixels(data.into(), width, height)
    }

    /// A decoded image over straight-alpha RGBA8 pixels an app shares
    /// ([`ImageLoadResult::Decoded`]): the same buffer, not a copy.
    pub fn new_shared(data: Arc<[u8]>, width: u32, height: u32) -> Self {
        Self::from_pixels(data.into(), width, height)
    }

    fn from_pixels(data: ImagePixels, width: u32, height: u32) -> Self {
        Self {
            data,
            width,
            height,
            vector: None,
            premultiplied: std::sync::OnceLock::new(),
            blob: std::sync::OnceLock::new(),
            id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    /// A decoded SVG file (#2111): no pixels of its own, rasterised per draw.
    pub fn from_vector(vector: crate::svg_image::VectorImage) -> Self {
        let (w, h) = vector.concrete();
        let mut image = Self::new(Vec::new(), w.round() as u32, h.round() as u32);
        image.vector = Some(Arc::new(vector));
        image
    }

    /// The SVG file this image is, if it is one.
    pub fn vector(&self) -> Option<&crate::svg_image::VectorImage> {
        self.vector.as_deref()
    }

    /// The image's natural width, height and ratio, which lay an `<img>` out
    /// and size an `auto` background. A raster image's pixel size; an SVG
    /// file's root `width`/`height`/`viewBox`, any of which may be absent.
    pub fn natural(&self) -> crate::svg_image::NaturalSize {
        match &self.vector {
            Some(v) => v.natural(),
            None => crate::svg_image::NaturalSize::of_pixels(self.width, self.height),
        }
    }

    /// This image's process-unique identity.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The pixels as one shared `peniko::Blob` (straight alpha), the same
    /// blob, and so the same Vello image, on every call.
    pub fn blob(&self) -> peniko::Blob<u8> {
        self.blob
            .get_or_init(|| peniko::Blob::new(self.data.0.clone()))
            .clone()
    }

    /// The pixels premultiplied, as the software painter draws them. Computed
    /// once per image and cached; `data` itself when every pixel is opaque.
    pub fn premultiplied(&self) -> &[u8] {
        let cached = self.premultiplied.get_or_init(|| {
            if self.data.chunks(4).all(|px| px.get(3) == Some(&255)) {
                None
            } else {
                Some(premultiply_rgba(&self.data))
            }
        });
        cached.as_deref().unwrap_or(&self.data)
    }
}

/// The inverse of [`premultiply_rgba`], for a painter that takes straight
/// alpha (the default [`Painter::fill_repeating`](crate::paint::painter::Painter::fill_repeating)).
pub fn unpremultiply_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(4) {
        let a = chunk[3];
        if a == 255 || a == 0 {
            out.extend_from_slice(if a == 0 { &[0, 0, 0, 0] } else { chunk });
        } else {
            let f = 255.0 / a as f32;
            for &c in &chunk[..3] {
                out.push((c as f32 * f + 0.5).min(255.0) as u8);
            }
            out.push(a);
        }
    }
    out
}

/// Straight-alpha RGBA8 to premultiplied, rounding to nearest: the
/// arithmetic the software painter's `draw_image` has always used, so a
/// cached copy draws exactly the pixels a per-draw premultiply would.
pub fn premultiply_rgba(data: &[u8]) -> Vec<u8> {
    let mut premul = Vec::with_capacity(data.len());
    for chunk in data.chunks(4) {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];
        if a == 255 {
            premul.extend_from_slice(&[r, g, b, a]);
        } else if a == 0 {
            premul.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let af = a as f32 / 255.0;
            premul.push((r as f32 * af + 0.5) as u8);
            premul.push((g as f32 * af + 0.5) as u8);
            premul.push((b as f32 * af + 0.5) as u8);
            premul.push(a);
        }
    }
    premul
}

/// State of an image in the cache.
enum ImageState {
    /// Image is currently being loaded/decoded on a background thread.
    ///
    /// `reload` is set when [`reload_image`] named this source while the load
    /// was in flight: the answer on its way was asked for *before* the app
    /// said the source changed, so it is discarded when it lands and the
    /// source is asked for once more.
    Loading { reload: bool },
    /// Image has been decoded and is ready to paint. Shared: every document
    /// showing the same source through the same loader holds this one image
    /// (#2757, [`shared_image`]).
    Decoded(Arc<DecodedImage>),
    /// Image loading or decoding failed.
    #[allow(dead_code)]
    Failed(String),
}

/// A completed image load result, pending insertion into the main cache.
pub struct PendingImage {
    /// Identity of the requesting document (`RinchDocument::doc_key`) — the
    /// queue is process-global, so entries must be tagged so each document
    /// drains only its own decodes (issue #137).
    pub doc_key: u64,
    pub src: String,
    /// The picture (shared by every document that asked for it, #2757) or
    /// why there is none.
    pub result: Result<Arc<DecodedImage>, String>,
}

/// Thread-safe queue for completed image loads from background threads.
///
/// Background threads push into this; the main thread drains it during layout.
/// Entries are tagged with the requesting document's `doc_key` — each
/// document's [`ImageCache::drain_pending`] removes only its own entries.
static PENDING_IMAGES: Mutex<Vec<PendingImage>> = Mutex::new(Vec::new());

/// Sources [`reload_image`] named, one entry per live document, waiting for
/// that document's next [`RinchDocument::drain_pending_images`](crate::RinchDocument::drain_pending_images).
///
/// Keyed by document, so asking whether a document has any, taking them and
/// purging them touch that document's entry alone, and each entry carries the
/// set of what it holds, so [`reload_image`] answers "is this source queued
/// already" with one lookup however many are (issue #1473: it scanned the
/// whole queue per live document, under this mutex). A document with nothing
/// queued has no entry: only [`reload_image`] makes one, only for a document
/// for a document in [`LIVE_DOCUMENTS`] whose cache holds the source, and the
/// drain and [`purge_pending`] remove it whole.
static PENDING_RELOADS: LazyLock<Mutex<HashMap<u64, ReloadQueue>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One document's undrained reloads: the sources in the order they were first
/// named, and the same sources as a set. Both are written by [`Self::push`]
/// alone and leave together, so they cannot disagree.
#[derive(Default)]
struct ReloadQueue {
    order: Vec<Arc<str>>,
    queued: HashSet<Arc<str>>,
}

impl ReloadQueue {
    /// Queue `src` unless it is queued already.
    fn push(&mut self, src: &str) {
        note_reload_queue_step();
        if !self.queued.contains(src) {
            let src: Arc<str> = Arc::from(src);
            self.queued.insert(src.clone());
            self.order.push(src);
        }
    }
}

/// The `doc_key` of every live [`RinchDocument`](crate::RinchDocument), with
/// the sources its [`ImageCache`] holds (`data:` ones aside), so a reload asked
/// for from any thread reaches each document that knows the source, and no
/// entry is ever queued for a document that can no longer drain it.
///
/// A document whose cache does not hold a source has nothing to reload for it
/// ([`ImageCache::begin_reload`] answers `false`), so it is not told (#1493):
/// a queued entry opens its frame gate ([`has_pending`]), which made every
/// other window or embedded context lay itself out and ask for a redraw on
/// each reload, to find nothing to do. A source joins the set the first time
/// the cache records it and stays for the document's life, as cache entries
/// do.
static LIVE_DOCUMENTS: LazyLock<Mutex<HashMap<u64, HashSet<Arc<str>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Cache of loaded images, keyed by source string (file path or URL).
pub struct ImageCache {
    entries: HashMap<String, ImageState>,
    /// The document this cache belongs to (`0`: none, as for a cache made on
    /// its own), whose entry in [`LIVE_DOCUMENTS`] records each source the
    /// cache comes to hold, so that [`reload_image`] reaches it.
    doc_key: u64,
    /// Sources whose in-flight answer was discarded by [`Self::drain_pending`]
    /// because a reload was asked for meanwhile: the document starts them again.
    retries: Vec<String>,
    /// The **decoded** sources whose reload is in flight (the entry stays
    /// `Decoded`, so the picture stays on screen). The value is the mark
    /// `Loading { reload }` carries for a source with no picture: a further
    /// reload was asked for while this one was out, so its answer is discarded
    /// when it lands and the source asked for once more. Removed when the
    /// reload's answer is taken.
    reloading: HashMap<String, bool>,
    /// Sources whose load [`Self::drain_pending`] found failed (and with no
    /// picture to keep): their `<img>`s now show a placeholder (#2309).
    newly_failed: Vec<String>,
    /// Failed sources whose reload is in flight ([`Self::shows_failure`]).
    /// Removed when the reload's answer is taken.
    retrying_failed: HashSet<String>,
    /// How many load answers (a picture, or a failure) [`Self::drain_pending`]
    /// has taken into the cache. A shell compares it to see that pictures
    /// arrived since it last painted a document it does not present: a hidden
    /// window it prepared (#2633).
    answers_taken: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageCache {
    /// Create a new empty image cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            doc_key: 0,
            retries: Vec::new(),
            reloading: HashMap::new(),
            newly_failed: Vec::new(),
            retrying_failed: HashSet::new(),
            answers_taken: 0,
        }
    }

    /// How many load answers this cache has taken in ([`Self::drain_pending`]).
    pub fn answers_taken(&self) -> u64 {
        self.answers_taken
    }

    /// Bind the cache to its document, so [`reload_image`] reaches the
    /// sources it comes to hold.
    pub(crate) fn set_doc_key(&mut self, doc_key: u64) {
        self.doc_key = doc_key;
    }

    /// Record `src` in `state`. Every entry is made here, so that a source
    /// new to the cache is noted in [`LIVE_DOCUMENTS`] for [`reload_image`].
    fn put(&mut self, src: String, state: ImageState) {
        if self.doc_key != 0 && !src.starts_with("data:") && !self.entries.contains_key(&src) {
            let mut live = LIVE_DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(held) = live.get_mut(&self.doc_key)
                && !held.contains(src.as_str())
            {
                held.insert(Arc::from(src.as_str()));
            }
        }
        self.entries.insert(src, state);
    }

    /// Get a decoded image if available.
    pub fn get(&self, src: &str) -> Option<&DecodedImage> {
        self.get_shared(src).map(|img| &**img)
    }

    /// [`Self::get`] as the shared handle the cache holds: the same `Arc` in
    /// every document showing `src` through the same loader (#2757).
    pub fn get_shared(&self, src: &str) -> Option<&Arc<DecodedImage>> {
        match self.entries.get(src) {
            Some(ImageState::Decoded(img)) => Some(img),
            _ => None,
        }
    }

    /// Check if a source is already in the cache (in any state).
    pub fn contains(&self, src: &str) -> bool {
        self.entries.contains_key(src)
    }

    /// Mark a source as currently loading.
    pub fn mark_loading(&mut self, src: String) {
        self.put(src, ImageState::Loading { reload: false });
    }

    /// Whether loading or decoding `src` failed (and nothing has asked for it
    /// again since).
    pub fn is_failed(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(ImageState::Failed(_)))
    }

    /// Whether `src` has no picture because its load failed, including while
    /// a reload of that failure is in flight: an `<img>` naming it keeps its
    /// placeholder (#2309) until the reload's answer lands, rather than
    /// blanking it for the reload (whose answer may well be another failure).
    pub fn shows_failure(&self, src: &str) -> bool {
        self.is_failed(src) || self.retrying_failed.contains(src)
    }

    /// Forget what is known about `src` so that it is asked for again: the
    /// cache half of [`reload_image`]. Returns whether the caller should start
    /// a load for it now.
    ///
    /// - A **failed** source goes back to loading: `true`.
    /// - A **decoded** one keeps its pixels on screen until the new answer
    ///   replaces them (no flash of an empty box): `true`. If the new load
    ///   fails, the pixels it had are kept.
    /// - One whose load is **in flight** — a first load, or the reload of a
    ///   failed or decoded source that has not been answered yet — is marked,
    ///   so the answer on its way is discarded and the source asked for once
    ///   more ([`Self::take_retries`]): `false`, nothing to start yet. However
    ///   many reloads are asked for while one load is out, one load follows
    ///   it, so the last answer taken was asked for after the last reload.
    /// - A source nothing has asked for has nothing to forget: `false`. It is
    ///   loaded fresh when an element first names it.
    ///
    /// A caller that gets `true` must start the load
    /// ([`request_image_load`]): the source counts as in flight until that
    /// load's answer is drained.
    pub fn begin_reload(&mut self, src: &str) -> bool {
        match self.entries.get_mut(src) {
            Some(state @ ImageState::Failed(_)) => {
                *state = ImageState::Loading { reload: false };
                self.retrying_failed.insert(src.to_string());
                true
            }
            Some(ImageState::Decoded(_)) => match self.reloading.get_mut(src) {
                Some(again) => {
                    *again = true;
                    false
                }
                None => {
                    self.reloading.insert(src.to_string(), false);
                    true
                }
            },
            Some(ImageState::Loading { reload }) => {
                *reload = true;
                false
            }
            None => false,
        }
    }

    /// The sources [`Self::drain_pending`] discarded an answer for because a
    /// reload was asked for while it was in flight. They are marked loading;
    /// the caller starts their loads.
    pub fn take_retries(&mut self) -> Vec<String> {
        std::mem::take(&mut self.retries)
    }

    /// The sources [`Self::drain_pending`] recorded as failed since the last
    /// call, each once per answer: what an `<img>` naming one draws changed
    /// from nothing to its placeholder (#2309).
    pub fn take_newly_failed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.newly_failed)
    }

    /// Insert a decoded image into the cache.
    pub fn insert_decoded(&mut self, src: String, image: DecodedImage) {
        self.insert_shared(src, Arc::new(image));
    }

    /// Take the picture another document shows for `src` (through the loader
    /// that answers it for a document whose own loader is `doc_loader`), if
    /// any: `true`, and it is in the cache now (#2757). `false` leaves `src`
    /// marked loading, and the caller starts its load ([`request_image_load`]).
    ///
    /// The source is recorded (and so noted in `LIVE_DOCUMENTS`) **before**
    /// the store is asked: a [`reload_image`] from another thread that lands
    /// after the lookup has found this document holding the source and queued
    /// a reload for it, which its next drain applies to the picture taken
    /// here. Looked up first, such a reload saw no holder and queued nothing,
    /// and the picture it replaced was kept for good (review of #2760).
    pub fn take_shared(&mut self, src: &str, doc_loader: &Arc<dyn ImageLoader>) -> bool {
        self.mark_loading(src.to_string());
        match shared_image(src, doc_loader) {
            Some(image) => {
                self.put(src.to_string(), ImageState::Decoded(image));
                true
            }
            None => false,
        }
    }

    /// Drop every entry: what a document lets go of as it is torn down, so
    /// that [`purge_pending`] finds the shared pictures only it held already
    /// dead and drops their slots (and loaders) with it.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// [`Self::insert_decoded`] for a picture another document may hold too.
    pub fn insert_shared(&mut self, src: String, image: Arc<DecodedImage>) {
        self.put(src, ImageState::Decoded(image));
    }

    /// Mark a source as failed.
    pub fn mark_failed(&mut self, src: String, error: String) {
        self.put(src, ImageState::Failed(error));
    }

    /// Take in an answer resolved in place ([`resolve_registered`]), as
    /// [`Self::drain_pending`] takes a load's: a picture is in the cache now; a
    /// failure is recorded and listed among the newly failed, so an `<img>`
    /// naming it shows its placeholder at the next drain (#2309), which comes
    /// before this frame's layout.
    pub(crate) fn take_resolved(&mut self, src: &str, answer: Result<Arc<DecodedImage>, String>) {
        match answer {
            Ok(image) => self.insert_shared(src.to_string(), image),
            Err(error) => {
                tracing::warn!("Image load failed for {src}: {error}");
                self.newly_failed.push(src.to_string());
                self.put(src.to_string(), ImageState::Failed(error));
            }
        }
    }

    /// Drain this document's entries from the pending images queue and insert
    /// them into this cache. Entries tagged with a different `doc_key` are left
    /// queued for their own document (issue #137).
    ///
    /// Returns the list of source strings that were newly decoded (for re-layout).
    pub fn drain_pending(&mut self, doc_key: u64) -> Vec<String> {
        let pending: Vec<PendingImage> = PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extract_if(.., |item| item.doc_key == doc_key)
            .collect();
        let mut newly_decoded = Vec::new();
        for item in pending {
            // A reload was asked for while this load was in flight: its answer
            // predates whatever changed, so it is dropped and asked for again.
            if let Some(ImageState::Loading { reload }) = self.entries.get_mut(&item.src)
                && *reload
            {
                *reload = false;
                self.retries.push(item.src);
                continue;
            }
            // The same for the reload of a picture that is on screen: a
            // further reload was asked for while this one was out. The retry
            // is the reload now in flight, with nothing asked for behind it.
            if let Some(again) = self.reloading.get_mut(&item.src) {
                if *again {
                    *again = false;
                    self.retries.push(item.src);
                    continue;
                }
                self.reloading.remove(&item.src);
            }
            self.retrying_failed.remove(&item.src);
            self.answers_taken += 1;
            match item.result {
                Ok(img) => {
                    newly_decoded.push(item.src.clone());
                    self.put(item.src, ImageState::Decoded(img));
                }
                Err(e) => {
                    tracing::warn!("Image load failed for {}: {}", item.src, e);
                    // A reload of a picture that is on screen keeps it there,
                    // unless the picture was taken away: a `register_rgba`
                    // name unregistered draws nothing, not a stale picture.
                    if !matches!(self.entries.get(&item.src), Some(ImageState::Decoded(_)))
                        || rinch_core::image::is_unregistered_rgba(&item.src)
                    {
                        self.newly_failed.push(item.src.clone());
                        self.put(item.src, ImageState::Failed(e));
                    }
                }
            }
        }
        newly_decoded
    }
}

/// Built-in image loader that reads from the local filesystem.
pub struct FileImageLoader;

impl ImageLoader for FileImageLoader {
    fn load(&self, src: &str) -> ImageLoadResult {
        let path = src.strip_prefix("file://").unwrap_or(src);
        match std::fs::read(path) {
            Ok(bytes) => ImageLoadResult::Loaded(bytes),
            Err(e) => ImageLoadResult::Failed(format!("Failed to read {}: {}", src, e)),
        }
    }
}

/// Decode encoded image bytes (PNG, JPEG, GIF, WebP) into straight-alpha
/// RGBA8, or parse an SVG file (#2111), which is recognised by its content
/// ([`crate::svg_image::sniff_svg`]).
pub fn decode_image_bytes(bytes: &[u8]) -> Result<DecodedImage, String> {
    decode_image_bytes_for(bytes, false)
}

/// [`decode_image_bytes`], told whether the source names an SVG
/// ([`crate::svg_image::source_says_svg`]: a `.svg` path or an
/// `image/svg+xml` data URL). Such bytes that the raster decoders do not know
/// are parsed as SVG too, so their failure says why the SVG did not parse.
pub fn decode_image_bytes_for(bytes: &[u8], says_svg: bool) -> Result<DecodedImage, String> {
    if crate::svg_image::sniff_svg(bytes) {
        return crate::svg_image::decode_svg(bytes).map(DecodedImage::from_vector);
    }
    match decode_raster(bytes) {
        Ok(rgba) => {
            let (w, h) = (rgba.width(), rgba.height());
            Ok(DecodedImage::new(rgba.into_raw(), w, h))
        }
        Err(_) if says_svg => crate::svg_image::decode_svg(bytes).map(DecodedImage::from_vector),
        Err(e) => Err(format!("Failed to decode image: {}", e)),
    }
}

/// Decode a raster file under rinch's limit: the `image` crate's own defaults
/// (`load_from_memory`), with the allocation limit spelled from
/// [`MAX_DECODED_IMAGE_BYTES`](rinch_core::image::MAX_DECODED_IMAGE_BYTES),
/// the one decoded pixels handed over by an app are held to as well.
///
/// The pixels come back turned the way the file's EXIF orientation says
/// (#2785, `image-orientation: from-image`, which rinch always applies:
/// Stylo's servo build does not parse `image-orientation`, #2802), so the decoded
/// picture's size is its oriented size and paint draws it as it is. Which
/// EXIF counts is Chrome's rule ([`crate::image_metadata`]); a file with none,
/// or orientation 1, is decoded exactly as before.
///
/// A file tagged with an ICC profile that is not sRGB comes back converted to
/// sRGB (#2779, [`crate::image_color`], feature `color-management`): at 16
/// bits a channel when it was decoded at 16 (#2807), and through its grey
/// curve when it is a grey picture with a grey profile; an untagged or
/// sRGB-tagged file is not touched.
fn decode_raster(bytes: &[u8]) -> image::ImageResult<image::RgbaImage> {
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(rinch_core::image::MAX_DECODED_IMAGE_BYTES);
    let format = image::guess_format(bytes)?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    reader.limits(limits.clone());
    // `ImageReader::decode`, with the decoder in hand to ask for its profile:
    // the output buffer is reserved against the limit, and the decoder is
    // then held to what is left of it for its own working memory.
    let mut decoder = reader.into_decoder()?;
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    #[cfg(feature = "color-management")]
    let to_srgb = if crate::image_color::color_management() {
        let icc = match format {
            // The image crate's JPEG decoder parses the headers again to
            // find it; the segments are read here instead, a walk over the
            // segment headers.
            image::ImageFormat::Jpeg => crate::image_color::jpeg_icc_profile(bytes),
            // Already parsed with the header (PNG `iCCP`, WebP `ICCP`).
            image::ImageFormat::Png | image::ImageFormat::WebP => {
                decoder.icc_profile().ok().flatten()
            }
            _ => None,
        };
        icc.and_then(|icc| crate::image_color::to_srgb(&icc))
    } else {
        None
    };
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    if let Some(o) = crate::image_metadata::exif_orientation(format, bytes) {
        crate::image_metadata::orient(&mut img, o);
    }
    #[cfg(feature = "color-management")]
    if let Some(t) = to_srgb {
        return Ok(crate::image_color::convert(t.as_ref(), img));
    }
    Ok(img.into_rgba8())
}

/// Take decoded pixels an app handed over ([`ImageLoadResult::Decoded`]):
/// checked ([`rinch_core::image::check_rgba`]), never decoded, never copied.
pub fn decoded_rgba_image(
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
) -> Result<DecodedImage, String> {
    rinch_core::image::check_rgba(width, height, rgba.len()).map_err(|e| e.to_string())?;
    Ok(DecodedImage::new_shared(rgba, width, height))
}

/// Decode a `data:` URL's image in place: no loader is ever offered one
/// (#2100). The `<img>` and `background-image` paths both come here.
pub fn decode_data_url_image(src: &str) -> Result<DecodedImage, String> {
    let bytes = decode_data_uri(src).ok_or_else(|| "malformed data: URL".to_string())?;
    decode_image_bytes_for(&bytes, crate::svg_image::source_says_svg(src))
}

/// Ask `loader` (already resolved: [`effective_loader`]) and decode its
/// answer: one load, wherever it runs.
///
/// Every load produces exactly one answer. A loader is app code (a closure
/// over a blob store), and one that panics must not take the answer with it:
/// the source would stay "loading" for the life of the document, and a reload
/// of a loading source only waits for the answer on its way. So where panics
/// unwind, a panic in the loader or the decode is a failed load, which a
/// reload starts again like any other failure.
fn load_and_decode(src: &str, loader: &dyn ImageLoader) -> Result<DecodedImage, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if src.starts_with("data:") {
            return decode_data_url_image(src);
        }
        // App code: untracked, so a signal it reads subscribes no effect
        // that happens to be running (CLAUDE.md, "Every app-callback invoke
        // runs untracked").
        match rinch_core::untracked_handler(|| loader.load(src)) {
            ImageLoadResult::Loaded(bytes) => {
                decode_image_bytes_for(&bytes, crate::svg_image::source_says_svg(src))
            }
            ImageLoadResult::Decoded {
                width,
                height,
                rgba,
            } => decoded_rgba_image(width, height, rgba),
            ImageLoadResult::Failed(e) => Err(e),
        }
    }))
    .unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        Err(format!("image loader panicked: {message}"))
    })
}

/// The loader that answers for `src`: a scheme the app installed a loader for
/// is the app's to answer (process-wide, [`rinch_core::image::image_scheme_loader`]);
/// everything else is the document's loader's.
fn effective_loader(src: &str, doc_loader: &Arc<dyn ImageLoader>) -> Arc<dyn ImageLoader> {
    rinch_core::image::image_scheme_loader(src).unwrap_or_else(|| doc_loader.clone())
}

/// The built-in file loader every document starts with, one instance for the
/// process: decoded pictures are shared by loader identity (#2757), so each
/// document having its own `FileImageLoader` would share nothing.
pub fn default_file_loader() -> Arc<dyn ImageLoader> {
    static FILE_LOADER: LazyLock<Arc<dyn ImageLoader>> =
        LazyLock::new(|| Arc::new(FileImageLoader));
    FILE_LOADER.clone()
}

// ---------------------------------------------------------------------------
// The process-wide store of decoded pictures (#2757).
//
// Each document keeps its own [`ImageCache`]: what it asked for, what is in
// flight for it, its reload marks. The pixels are not its own. A load is keyed
// by its source **and the loader that answers it** (by the loader's identity,
// the `Arc`'s address), and every document asking the same loader for the same
// source while a decode of it is alive gets that one decode: an
// `Arc<DecodedImage>`, so its premultiplied copy, its GPU blob and its mipmap
// levels (keyed by its id, `image_minify`) are shared too.
//
// - **Ready**: the store holds a `Weak`; the documents' caches hold the `Arc`s.
//   When the last document lets go the picture is freed, and the slot is
//   dropped at the next request for that source or the next document drop.
// - **Loading**: one load is running; a document that asks meanwhile is added
//   to its waiters and gets the same answer (in-flight dedup). Failures are
//   handed to the waiters but not kept: a later request asks again, as it did
//   before the store.
// - **Reload** ([`reload_image`], or one document's own
//   [`RinchDocument::reload_image`](crate::RinchDocument::reload_image)): the
//   source's slots are forgotten, so the next request loads it fresh. A load in
//   flight still answers the documents that were waiting for it (each
//   document's cache then discards an answer that predates its reload, as
//   before), but nobody joins it any more and its answer is not kept.
//
// A document whose loader is its own (an app that set
// `tree.image_loader` per window) shares only with documents holding the same
// `Arc`: a loader that could answer differently per document is never asked
// once for two of them. Scheme loaders are process-wide by construction, and
// the default file loader is one instance ([`default_file_loader`]).
// ---------------------------------------------------------------------------

/// One loader's state for one source.
struct Slot {
    /// The loader this slot is keyed by. Held so its address cannot be
    /// reused by another loader while the slot exists.
    loader: Arc<dyn ImageLoader>,
    state: SlotState,
}

enum SlotState {
    Loading { flight: u64, waiters: Vec<u64> },
    Ready(Weak<DecodedImage>),
}

impl Slot {
    fn is_for(&self, loader: &Arc<dyn ImageLoader>) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.loader), Arc::as_ptr(loader))
    }

    fn is_dead(&self) -> bool {
        matches!(&self.state, SlotState::Ready(w) if w.strong_count() == 0)
    }
}

#[derive(Default)]
struct SharedImages {
    /// The slots by source; a source shown through several loaders has one
    /// slot per loader.
    by_src: HashMap<String, Vec<Slot>>,
    /// The waiters of loads in flight whose source was reloaded meanwhile:
    /// they still get its answer, which is not kept.
    superseded: HashMap<u64, Vec<u64>>,
    next_flight: u64,
}

impl SharedImages {
    /// The live picture `loader` gave for `src`, if a document still holds it.
    fn ready(&self, src: &str, loader: &Arc<dyn ImageLoader>) -> Option<Arc<DecodedImage>> {
        self.by_src
            .get(src)?
            .iter()
            .find(|slot| slot.is_for(loader))
            .and_then(|slot| match &slot.state {
                SlotState::Ready(weak) => weak.upgrade(),
                SlotState::Loading { .. } => None,
            })
    }
}

static SHARED_IMAGES: LazyLock<Mutex<SharedImages>> =
    LazyLock::new(|| Mutex::new(SharedImages::default()));

fn shared_images() -> std::sync::MutexGuard<'static, SharedImages> {
    SHARED_IMAGES.lock().unwrap_or_else(|e| e.into_inner())
}

/// How many loads the store has started, process-wide (a test's count).
static LOADS_STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many image loads have been started in this process: one per source and
/// loader however many documents ask at once or while it is shown (#2757).
#[doc(hidden)]
pub fn loads_started() -> u64 {
    LOADS_STARTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The decoded pictures the store knows to be alive, and the bytes of their
/// pixels: what all documents together hold, each picture counted once.
#[doc(hidden)]
pub fn shared_images_held() -> (usize, usize) {
    let shared = shared_images();
    let mut seen = HashSet::new();
    let mut bytes = 0;
    for slot in shared.by_src.values().flatten() {
        if let SlotState::Ready(weak) = &slot.state
            && let Some(image) = weak.upgrade()
            && seen.insert(image.id())
        {
            bytes += image.data.len();
        }
    }
    (seen.len(), bytes)
}

/// The picture another document already shows for `src` through the loader
/// that answers it for a document whose own loader is `doc_loader`, if any:
/// a document asking for it takes this instead of loading it again (#2757).
///
/// A document takes it through [`ImageCache::take_shared`], which records the
/// source first: a picture looked up here and put in a cache afterwards misses
/// a reload that lands in between.
///
/// A `data:` URL is not shared: it is decoded in place, per document, and
/// none is ever in the store.
pub fn shared_image(src: &str, doc_loader: &Arc<dyn ImageLoader>) -> Option<Arc<DecodedImage>> {
    if src.starts_with("data:") {
        return None;
    }
    let loader = effective_loader(src, doc_loader);
    shared_images().ready(src, &loader)
}

/// The answer for `src` now, when the built-in [`register_rgba`] registry is
/// what answers it (#2857): its pixels are decoded and in memory already, so a
/// document takes them in the frame that names the source, with no load
/// thread and no wait for the next drain. An element whose picture moves from
/// one registered name to another then draws the new picture in the frame that
/// applies the change, and an unknown or unregistered name fails in that frame
/// too. `None` for any other source, and for a `rinch-image:` source an app's
/// own scheme loader has taken over: those load as before.
///
/// The picture goes through the shared store as a load's would (#2757): a
/// picture another document already shows under `src` is that one, and one
/// made here is kept for the next document that asks, while one shows it. A
/// load of `src` still in flight (a reload) keeps its slot; this answer is
/// then the document's own until that load lands.
///
/// The caller marks `src` loading in its cache before it calls this
/// ([`ImageCache::mark_loading`]), and starts an ordinary load
/// ([`request_image_load`]) when this answers `None`.
///
/// [`register_rgba`]: rinch_core::image::register_rgba
pub(crate) fn resolve_registered(src: &str) -> Option<Result<Arc<DecodedImage>, String>> {
    let mut graveyard = Vec::new();
    let result = {
        // The registry is read under the store's lock, as a load's answer is
        // kept under it (`finish_load`): a reload of `src` from another
        // thread (`register_rgba`, `unregister_rgba`) forgets the store's
        // slot (`forget_shared`) either before this read, which then sees
        // the new pixels, or after the slot is pushed. The caller has
        // recorded the source in its cache first (`ImageCache::mark_loading`),
        // so a reload that comes after finds this document holding it and
        // queues one for it (review of #2860, as #2760's was for
        // `take_shared`). Both are needed: with the record after the read,
        // the reload's `forget_shared` and its scan of `LIVE_DOCUMENTS` can
        // fall between this push and the record, and miss the document; with
        // the read before the lock, a whole `register_rgba` can fall between
        // the read and the push, and its reload, queued for the document,
        // finds the stale slot pushed after its `forget_shared` and serves it
        // again.
        //
        // Lock order: store, then registry; never the reverse. This holds the
        // store's lock while `registered_rgba_answer` takes `SCHEME_LOADERS`
        // and `RGBA_IMAGES` for reading. `register_rgba` and
        // `unregister_rgba` let go of `RGBA_IMAGES` before `reload_image`,
        // which copies the reloaders out of `RELOADERS` before calling them;
        // the reloader here takes the store's lock (`forget_shared`) and,
        // after letting it go, `LIVE_DOCUMENTS` and `PENDING_RELOADS`.
        // `register_image_scheme` takes `SCHEME_LOADERS` alone. No path holds
        // the registry's lock or the app loaders' while taking the store's
        // (`registered_rgba_two_threads_2860_tests` would hang on one).
        let mut shared = shared_images();
        let (loader, answer) = rinch_core::image::registered_rgba_answer(src)?;
        let slots = shared.by_src.entry(src.to_string()).or_default();
        bury_dead(slots, &mut graveyard);
        let held = slots.iter().find(|slot| slot.is_for(&loader));
        if let Some(Slot {
            state: SlotState::Ready(weak),
            ..
        }) = held
            && let Some(image) = weak.upgrade()
        {
            return Some(Ok(image));
        }
        let in_flight = held.is_some();
        let result = match answer {
            ImageLoadResult::Decoded {
                width,
                height,
                rgba,
            } => decoded_rgba_image(width, height, rgba).map(Arc::new),
            ImageLoadResult::Failed(error) => Err(error),
            ImageLoadResult::Loaded(_) => Err(format!("the {src} registry answered encoded bytes")),
        };
        if let Ok(image) = &result
            && !in_flight
        {
            slots.push(Slot {
                loader,
                state: SlotState::Ready(Arc::downgrade(image)),
            });
        }
        if slots.is_empty() {
            shared.by_src.remove(src);
        }
        result
    };
    // Dead slots (and the loaders they hold) drop outside the store's lock.
    drop(graveyard);
    Some(result)
}

/// Forget what the store knows of `src`, through every loader, so that the
/// next request loads it fresh: the store's half of a reload. Documents keep
/// the pictures they hold until their own reload replaces them.
pub(crate) fn forget_shared(src: &str) {
    let mut shared = shared_images();
    let Some(mut slots) = shared.by_src.remove(src) else {
        return;
    };
    for slot in &mut slots {
        if let SlotState::Loading { flight, waiters } = &mut slot.state {
            shared.superseded.insert(*flight, std::mem::take(waiters));
        }
    }
    drop(shared);
    // The slots (and the loaders they hold, app code) drop outside the lock.
    drop(slots);
}

/// Move `slots`' dead ones into `graveyard`, to be dropped once the store's
/// lock is released (a slot holds a loader, app code whose drop may run
/// anything).
fn bury_dead(slots: &mut Vec<Slot>, graveyard: &mut Vec<Slot>) {
    graveyard.extend(slots.extract_if(.., |slot| slot.is_dead()));
}

/// What a request for a source found in the store.
enum Request {
    /// A document shows it: here it is.
    Ready(Arc<DecodedImage>),
    /// A load of it is running; the requester waits for it.
    Joined,
    /// Nothing: this load, numbered, is to be started.
    Start(u64),
}

/// Ask the store for `src` through `loader` on behalf of `doc_key`.
fn request_shared(doc_key: u64, src: &str, loader: &Arc<dyn ImageLoader>) -> Request {
    let mut graveyard = Vec::new();
    let request = request_shared_locked(doc_key, src, loader, &mut graveyard);
    drop(graveyard);
    request
}

fn request_shared_locked(
    doc_key: u64,
    src: &str,
    loader: &Arc<dyn ImageLoader>,
    graveyard: &mut Vec<Slot>,
) -> Request {
    let mut shared = shared_images();
    let SharedImages {
        by_src,
        next_flight,
        ..
    } = &mut *shared;
    let flight = *next_flight;
    let slots = by_src.entry(src.to_string()).or_default();
    bury_dead(slots, graveyard);
    match slots.iter_mut().find(|slot| slot.is_for(loader)) {
        Some(Slot {
            state: SlotState::Ready(weak),
            ..
        }) => match weak.upgrade() {
            Some(image) => Request::Ready(image),
            None => unreachable!("dead slots were dropped above"),
        },
        Some(Slot {
            state: SlotState::Loading { waiters, .. },
            ..
        }) => {
            waiters.push(doc_key);
            Request::Joined
        }
        None => {
            slots.push(Slot {
                loader: loader.clone(),
                state: SlotState::Loading {
                    flight,
                    waiters: vec![doc_key],
                },
            });
            *next_flight += 1;
            LOADS_STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Request::Start(flight)
        }
    }
}

/// A load numbered `flight` of `src` through `loader` has answered: keep the
/// picture (unless the source was reloaded meanwhile) and queue the answer for
/// every document that was waiting for it. Returns whether any was queued.
fn finish_load(
    flight: u64,
    src: &str,
    loader: &Arc<dyn ImageLoader>,
    result: Result<DecodedImage, String>,
) -> bool {
    let answer = result.map(Arc::new);
    let mut graveyard = Vec::new();
    let waiters = {
        let mut shared = shared_images();
        let mut waiters = None;
        if let Some(slots) = shared.by_src.get_mut(src)
            && let Some(at) = slots.iter().position(|slot| {
                slot.is_for(loader)
                    && matches!(slot.state, SlotState::Loading { flight: f, .. } if f == flight)
            })
        {
            let slot = &mut slots[at];
            let state = match &answer {
                // Kept for the next document that asks, while one shows it.
                Ok(image) => SlotState::Ready(Arc::downgrade(image)),
                // Not kept: a later request asks again.
                Err(_) => SlotState::Ready(Weak::new()),
            };
            if let SlotState::Loading { waiters: w, .. } = std::mem::replace(&mut slot.state, state)
            {
                waiters = Some(w);
            }
            bury_dead(slots, &mut graveyard);
            if slots.is_empty() {
                shared.by_src.remove(src);
            }
        }
        waiters
            .or_else(|| shared.superseded.remove(&flight))
            .unwrap_or_default()
    };
    drop(graveyard);
    let mut queued = false;
    for doc_key in waiters {
        queued |= queue_answer(doc_key, src.to_string(), answer.clone());
    }
    queued
}

/// Queue a load's answer for `doc_key`'s next drain, if the document is still
/// there: an answer queued for a dropped one is never drained. The live list
/// is held across the push, and [`purge_pending`] leaves it first, so a
/// document that drops meanwhile either purges this answer or stops it here.
fn queue_answer(doc_key: u64, src: String, result: Result<Arc<DecodedImage>, String>) -> bool {
    let live = LIVE_DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
    if !live.contains_key(&doc_key) {
        return false;
    }
    PENDING_IMAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(PendingImage {
            doc_key,
            src,
            result,
        });
    true
}

/// Wake the main thread for an answer just queued.
///
/// The desktop event loop runs on `ControlFlow::Wait`, so without this the
/// decode sits in the queue until some other input happens to arrive — an
/// `<img src>` set while the app is otherwise idle stays 0x0 and unpainted, in
/// practice forever. Dispatching an empty callback is the cheapest thing that
/// goes through the platform wake path.
///
/// The wake is *promptness only*, never the correctness guarantee, and must
/// not be treated as one: hosts install their own dispatcher, and an embedded
/// one deliberately wakes nothing (its game loop is already turning); the
/// desktop one coalesces, so a wake can be swallowed when another closure is
/// already queued. What actually guarantees the drain is [`has_pending`],
/// which every host's "is there anything to do?" gate consults.
fn wake_main_thread() {
    rinch_core::run_on_main_thread(|| {});
}

/// The loads [`request_image_load`] started on wasm32, waiting for
/// [`run_queued_loads`]: `(flight, src, loader)`.
#[cfg(target_arch = "wasm32")]
type QueuedLoad = (u64, String, Arc<dyn ImageLoader>);
#[cfg(target_arch = "wasm32")]
static QUEUED_LOADS: Mutex<Vec<QueuedLoad>> = Mutex::new(Vec::new());

/// Load `src` for document `doc_key` through `loader` (the document's own;
/// an app scheme loader takes over for its scheme), and queue the answer for
/// the document's next [`ImageCache::drain_pending`].
///
/// The load is shared (#2757): a picture another document shows from the same
/// source and loader is answered at once with that picture, and a request
/// while a load of it is running waits for that load. Only otherwise does a
/// load start.
///
/// On native targets the load runs on a background thread. `wasm32` has no OS
/// threads (`std::thread::spawn` panics on `wasm32-unknown-unknown` and fails
/// on `wasm32-wasip1`), so a load runs on the UI thread. Not here, though:
/// this is reached from inside the document's borrow
/// (`NodeHandle::set_attribute`, `resolve_layout`), and a loader is app code
/// that may write a signal, whose effects write the DOM and would borrow the
/// document again (review of #2107). The host runs the queue between its
/// borrows ([`run_queued_loads`]): `RinchContext::update` and
/// `RinchContext::new` do. Before #2100 this was a no-op there: no scheme
/// loader was asked and no `background-image` loaded in an embedded context
/// in the browser.
pub fn request_image_load(doc_key: u64, src: String, loader: Arc<dyn ImageLoader>) {
    let loader = effective_loader(&src, &loader);
    let flight = match request_shared(doc_key, &src, &loader) {
        Request::Ready(image) => {
            if queue_answer(doc_key, src, Ok(image)) {
                wake_main_thread();
            }
            return;
        }
        Request::Joined => return,
        Request::Start(flight) => flight,
    };
    start_load(flight, src, loader);
}

#[cfg(target_arch = "wasm32")]
fn start_load(flight: u64, src: String, loader: Arc<dyn ImageLoader>) {
    QUEUED_LOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((flight, src, loader));
}

#[cfg(not(target_arch = "wasm32"))]
fn start_load(flight: u64, src: String, loader: Arc<dyn ImageLoader>) {
    // 8 MB of stack, not the 2 MB a spawned thread gets: a decoder recurses
    // over its input (#2111's review: a nested SVG overflowed 2 MB, and a
    // stack overflow aborts the process). The SVG limits keep well inside it.
    let thread_src = src.clone();
    let thread_loader = loader.clone();
    let spawned = std::thread::Builder::new()
        .name("rinch-image-load".into())
        .stack_size(8 << 20)
        .spawn(move || {
            let result = load_and_decode(&thread_src, &*thread_loader);
            if finish_load(flight, &thread_src, &thread_loader, result) {
                wake_main_thread();
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("image load thread not started: {e}");
        // Every load produces exactly one answer, or its waiters would stay
        // "loading" for good.
        finish_load(
            flight,
            &src,
            &loader,
            Err(format!("image load thread not started: {e}")),
        );
    }
}

/// Run every image load requested since the last call, on this thread, and
/// queue each answer for the documents waiting for it. Returns whether any
/// ran.
///
/// On wasm32 a load is not run where it is requested (see
/// [`request_image_load`]): a host that drives a [`RinchDocument`](crate::RinchDocument)
/// itself calls this **with no document borrowed**, before the layout that
/// should show the pictures, and lays out again when it returns `true` and
/// the layout requested more (a background's URL is requested by the cascade
/// inside layout). `RinchContext` does this in `new` and every `update`, so a
/// loader that answers at once is shown in the frame that asked.
///
/// The loader (the app's scheme loader, else the document's) and the decode
/// run here, so the cost is the frame's: a loader must answer from memory, and
/// a large picture is decoded inside the frame. A loader may write signals.
///
/// On native targets loads run on their own threads and this does nothing.
pub fn run_queued_loads() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let mut ran = false;
        loop {
            // Taken before running: a loader may (through an effect) request
            // another load, which waits for the next round.
            let loads =
                std::mem::take(&mut *QUEUED_LOADS.lock().unwrap_or_else(|e| e.into_inner()));
            if loads.is_empty() {
                return ran;
            }
            for (flight, src, loader) in loads {
                let result = load_and_decode(&src, &*loader);
                finish_load(flight, &src, &loader, result);
                ran = true;
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

/// Whether any decode is queued for this document.
///
/// A completed image load dirties no DOM node — the decoding thread has no idea
/// which nodes reference the source, and [`ImageCache::drain_pending`] is what
/// works that out — so a frame loop that short-circuits on "nothing is dirty"
/// skips the drain and the image never gets its intrinsic size. Anything that
/// gates layout on dirtiness has to ask this too.
pub fn has_pending(doc_key: u64) -> bool {
    // A load still queued for the host to run (wasm32) that this document
    // waits for, possibly one another document started.
    #[cfg(target_arch = "wasm32")]
    {
        let shared = shared_images();
        let waits = shared.by_src.values().flatten().any(|slot| {
            matches!(&slot.state, SlotState::Loading { waiters, .. } if waiters.contains(&doc_key))
        }) || shared.superseded.values().any(|w| w.contains(&doc_key));
        if waits {
            return true;
        }
    }
    PENDING_IMAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|item| item.doc_key == doc_key)
        || PENDING_RELOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&doc_key)
}

/// Ask every document to load `src` again: the way a picture that failed to
/// load, or whose bytes have changed, gets another go without a restart. The
/// desktop half of [`rinch_core::image::reload_image`], which is what an app
/// calls (as `rinch::image::reload_image`) so that the same line works in a
/// browser build.
///
/// A load's answer is cached by source string, **failures included**: once a
/// loader has answered [`ImageLoadResult::Failed`] for a source, no element
/// that names it asks again. An app whose loader can answer late (a file that
/// is still downloading, a blob that has not been synced yet) calls this when
/// the bytes exist, and every `<img>` and `background-image` naming `src`, in
/// every window, picks the picture up: its box takes the decoded size and it
/// is painted, as for a first load.
///
/// Callable from **any thread**; it queues the request and wakes the UI
/// thread, and each document acts on it at its next layout. What happens per
/// document depends on what it knew ([`ImageCache::begin_reload`]): a failed
/// source is loaded again; a decoded one is loaded again and keeps showing its
/// old pixels until the new ones land; one whose load is still in flight (a
/// first load, or an earlier reload) has that answer discarded and is asked
/// for once more, so an answer that was already on its way cannot beat the
/// reload, and any number of reloads asked for while one load is out cost one
/// more load; a source no element has asked for is left alone (it loads fresh
/// when one does) — and is not even queued for that document, so a reload
/// wakes only the documents that hold the source (#1493). `data:` sources
/// never change and are ignored.
///
/// `src` is the cache key: it must be the source **exactly as the element or
/// stylesheet spelled it**, scheme case included (`MyApp:x` and `myapp:x` go
/// to one loader and are two sources).
pub fn reload_image(src: &str) {
    if src.is_empty() || src.starts_with("data:") {
        return;
    }
    // The next request loads it fresh, so the documents' reloads below share
    // one new load (#2757).
    forget_shared(src);
    {
        let live = LIVE_DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
        let mut reloads = PENDING_RELOADS.lock().unwrap_or_else(|e| e.into_inner());
        for (&doc_key, held) in live.iter() {
            if held.contains(src) {
                reloads.entry(doc_key).or_default().push(src);
            }
        }
    }
    // Promptness only, as for a finished decode: `has_pending` is what makes
    // every host's frame gate come round to the drain.
    rinch_core::run_on_main_thread(|| {});
}

thread_local! {
    /// See [`reload_queue_steps`].
    static RELOAD_QUEUE_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn note_reload_queue_step() {
    RELOAD_QUEUE_STEPS.with(|steps| steps.set(steps.get() + 1));
}

/// How many times [`reload_image`] has asked, on the calling thread, whether
/// a source was already queued for a document: the cost of its
/// de-duplication as a count (issue #1473). One set lookup per live document
/// that holds the source (#1493), per call, where it was one per queue entry
/// scanned.
#[doc(hidden)]
pub fn reload_queue_steps() -> u64 {
    RELOAD_QUEUE_STEPS.with(|steps| steps.get())
}

/// How many sources [`reload_image`] has queued for this document and its
/// drain has not taken yet.
#[doc(hidden)]
pub fn pending_reload_count(doc_key: u64) -> usize {
    PENDING_RELOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&doc_key)
        .map_or(0, |queue| queue.order.len())
}

/// Take the sources [`reload_image`] queued for this document.
///
/// In the order they were first named. The document's entry goes whole, so a
/// source named again after this is queued again.
pub(crate) fn take_pending_reloads(doc_key: u64) -> Vec<Arc<str>> {
    PENDING_RELOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&doc_key)
        .map(|queue| queue.order)
        .unwrap_or_default()
}

/// Note a document as live, so [`reload_image`] reaches it. Undone by
/// [`purge_pending`] when the document drops.
pub(crate) fn register_document(doc_key: u64) {
    // The first document also makes this backend answer the cross-target
    // `rinch_core::image::reload_image`.
    static REGISTER_RELOADER: std::sync::Once = std::sync::Once::new();
    REGISTER_RELOADER.call_once(|| rinch_core::image::add_image_reloader(reload_image));
    LIVE_DOCUMENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(doc_key)
        .or_default();
}

/// Remove all queued entries for a document that is being torn down.
///
/// Without this, decodes that land after a document is dropped would strand in
/// the process-global queue forever (nothing drains a dead doc_key). Called
/// from `RinchDocument::drop` (issue #137).
pub fn purge_pending(doc_key: u64) {
    // The live list first: a load thread that finishes from here on finds the
    // document gone and queues nothing; one that queued before is purged below.
    LIVE_DOCUMENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&doc_key);
    PENDING_IMAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|item| item.doc_key != doc_key);
    PENDING_RELOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&doc_key);
    // No longer waiting for any load (one still running answers the others,
    // or nobody), and the slots of pictures nobody holds any more go.
    let mut graveyard = Vec::new();
    let mut guard = shared_images();
    let shared = &mut *guard;
    shared.by_src.retain(|_, slots| {
        for slot in slots.iter_mut() {
            if let SlotState::Loading { waiters, .. } = &mut slot.state {
                waiters.retain(|&w| w != doc_key);
            }
        }
        bury_dead(slots, &mut graveyard);
        !slots.is_empty()
    });
    shared.superseded.retain(|_, waiters| {
        waiters.retain(|&w| w != doc_key);
        !waiters.is_empty()
    });
    // On wasm32 a load the host has not run yet, which nobody waits for any
    // more, is not run: its slot goes with it (a request from here on starts
    // a load of its own). A native load is already running on its thread.
    #[cfg(target_arch = "wasm32")]
    {
        let mut queued = QUEUED_LOADS.lock().unwrap_or_else(|e| e.into_inner());
        let orphaned: HashSet<u64> = queued
            .iter()
            .map(|load| load.0)
            .filter(|flight| {
                !shared.superseded.contains_key(flight)
                    && !shared.by_src.values().flatten().any(|slot| {
                        matches!(&slot.state, SlotState::Loading { flight: f, waiters }
                            if f == flight && !waiters.is_empty())
                    })
            })
            .collect();
        if !orphaned.is_empty() {
            let gone: Vec<QueuedLoad> = queued
                .extract_if(.., |load| orphaned.contains(&load.0))
                .collect();
            drop(queued);
            shared.by_src.retain(|_, slots| {
                graveyard.extend(slots.extract_if(.., |slot| {
                    matches!(&slot.state, SlotState::Loading { flight, .. } if orphaned.contains(flight))
                }));
                !slots.is_empty()
            });
            drop(guard);
            drop(gone);
            drop(graveyard);
            return;
        }
    }
    drop(guard);
    drop(graveyard);
}

/// Decode a `data:` URI into raw bytes.
///
/// `data:[<mediatype>][;base64],<data>`. The data is percent-decoded first,
/// as the fetch standard does; then, with `;base64`, decoded by its
/// forgiving base64: ASCII white space is ignored and padding is optional
/// (as Chrome decodes a `data:` URL). Without it, the percent-decoded bytes
/// are the data — the form an inline `data:image/svg+xml,%3Csvg…` takes
/// (#2111; before, only padded base64 decoded). Returns `None` if the URI has
/// no comma or its base64 is invalid.
pub fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let comma = src.find(',')?;
    let header = &src[..comma];
    let data = percent_decode(&src.as_bytes()[comma + 1..]);
    let base64 = header
        .split(';')
        .skip(1)
        .any(|p| p.trim().eq_ignore_ascii_case("base64"));
    if !base64 {
        return Some(data);
    }
    use base64::Engine;
    let forgiving = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let data: Vec<u8> = data
        .into_iter()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    forgiving.decode(data).ok()
}

/// `%XX` escapes to their bytes; a `%` not followed by two hex digits is kept
/// as it is (the URL standard's percent-decode).
fn percent_decode(input: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                input.get(i + 1).copied().and_then(hex),
                input.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod reload_queue_tests {
    //! The reload queue's own contract (issue #1473), on document keys no
    //! `RinchDocument` has: other tests' documents are live in this binary.
    //! A reload reaches every live document, so these run one at a time and
    //! each reads back only the sources it named (`q<n>:`).
    use super::*;

    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// A key registered as live for the test, and purged when it ends.
    struct Live(u64, &'static str);
    impl Live {
        fn new(n: u64, mine: &'static str) -> Self {
            let key = u64::MAX - 0x1473_0000 - n;
            register_document(key);
            Live(key, mine)
        }
        /// Say this key's (absent) cache holds `srcs`, as an `ImageCache`
        /// bound to it would record them: a reload reaches only a document
        /// that holds the source (#1493).
        fn holding(self, srcs: &[&str]) -> Self {
            let mut live = LIVE_DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
            let held = live.get_mut(&self.0).expect("registered");
            for src in srcs {
                held.insert(Arc::from(*src));
            }
            drop(live);
            self
        }
        fn taken(&self) -> Vec<String> {
            take_pending_reloads(self.0)
                .iter()
                .filter(|s| s.starts_with(self.1))
                .map(|s| s.to_string())
                .collect()
        }
        fn queued(&self) -> usize {
            let reloads = PENDING_RELOADS.lock().unwrap_or_else(|e| e.into_inner());
            reloads.get(&self.0).map_or(0, |queue| {
                assert_eq!(queue.order.len(), queue.queued.len(), "in step");
                queue.order.iter().filter(|s| s.starts_with(self.1)).count()
            })
        }
    }
    impl Drop for Live {
        fn drop(&mut self) {
            purge_pending(self.0);
        }
    }

    #[test]
    fn sources_are_taken_in_the_order_first_named_each_once() {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let doc = Live::new(1, "q1:").holding(&["q1:a", "q1:b", "q1:c"]);
        for src in ["q1:c", "q1:a", "q1:c", "q1:b", "q1:a", "q1:c"] {
            reload_image(src);
        }
        assert_eq!(doc.queued(), 3);
        assert_eq!(doc.taken(), ["q1:c", "q1:a", "q1:b"]);
        assert!(!has_pending(doc.0), "the take empties it");
        assert_eq!(doc.taken(), Vec::<String>::new());
    }

    /// The set leaves with the queue: a source taken by one drain and named
    /// again is queued again, not mistaken for still queued.
    #[test]
    fn a_source_named_again_after_a_drain_is_queued_again() {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let doc = Live::new(2, "q2:").holding(&["q2:a", "q2:b"]);
        reload_image("q2:a");
        reload_image("q2:b");
        assert_eq!(doc.taken(), ["q2:a", "q2:b"]);
        reload_image("q2:b");
        assert!(has_pending(doc.0));
        assert_eq!(doc.taken(), ["q2:b"]);
    }

    #[test]
    fn each_document_holds_and_gives_up_its_own() {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let a = Live::new(3, "q3:").holding(&["q3:x", "q3:y"]);
        let b = Live::new(4, "q3:").holding(&["q3:x", "q3:y"]);
        reload_image("q3:x");
        assert_eq!(b.queued(), 1, "control: it was queued for both");
        let gone = b.0;
        drop(b);
        assert_eq!(pending_reload_count(gone), 0, "purged with the document");
        assert!(!has_pending(gone));
        reload_image("q3:y");
        assert_eq!(pending_reload_count(gone), 0, "and never queued for again");
        assert_eq!(a.taken(), ["q3:x", "q3:y"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push an entry directly onto the process-global pending queue,
    /// bypassing the background-thread decode pipeline.
    fn push_pending(doc_key: u64, src: &str) {
        PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PendingImage {
                doc_key,
                src: src.to_string(),
                result: Ok(Arc::new(DecodedImage::new(vec![0; 4], 1, 1))),
            });
    }

    /// Count queued entries for a doc_key. Scoped per-key (not a global count)
    /// so parallel tests sharing the static queue don't interfere.
    fn pending_count_for(doc_key: u64) -> usize {
        PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|item| item.doc_key == doc_key)
            .count()
    }

    #[test]
    fn drain_pending_takes_only_own_documents_entries() {
        let doc1 = rinch_core::dom::next_doc_key();
        let doc2 = rinch_core::dom::next_doc_key();
        let mut cache1 = ImageCache::new();
        let mut cache2 = ImageCache::new();

        // A decode lands for doc2, then doc1 lays out first (the #137 race).
        push_pending(doc2, "doc2.png");
        assert!(
            cache1.drain_pending(doc1).is_empty(),
            "doc1 must not absorb doc2's decode"
        );
        assert!(cache1.get("doc2.png").is_none());
        assert_eq!(
            pending_count_for(doc2),
            1,
            "doc2's entry must survive doc1's drain"
        );

        // doc2's own drain picks it up.
        assert_eq!(cache2.drain_pending(doc2), vec!["doc2.png".to_string()]);
        assert!(cache2.get("doc2.png").is_some());
        assert_eq!(pending_count_for(doc2), 0);
    }

    #[test]
    fn has_pending_is_scoped_to_the_document_and_cleared_by_the_drain() {
        let doc1 = rinch_core::dom::next_doc_key();
        let doc2 = rinch_core::dom::next_doc_key();

        assert!(!has_pending(doc1), "nothing queued yet");

        // A decode lands for doc2. It is doc2's reason to resolve, and must not
        // become doc1's — a doc1 frame loop that believed this would resolve
        // every frame for ever, since doc1's drain can never clear it.
        push_pending(doc2, "doc2.png");
        assert!(has_pending(doc2));
        assert!(!has_pending(doc1));

        // The drain is what clears it — this is the loop-termination argument
        // for every gate that consults `has_pending`.
        let mut cache2 = ImageCache::new();
        cache2.drain_pending(doc2);
        assert!(!has_pending(doc2));
    }

    #[test]
    fn has_pending_covers_a_failed_decode_and_is_cleared_by_it() {
        // A failed load is queued like any other result, so it must also report
        // as pending: the drain is the only thing that removes it, and a gate
        // that ignored failures would leave the entry queued for ever.
        let doc = rinch_core::dom::next_doc_key();
        PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(PendingImage {
                doc_key: doc,
                src: "broken.png".to_string(),
                result: Err("nope".to_string()),
            });
        assert!(has_pending(doc));

        let mut cache = ImageCache::new();
        assert!(
            cache.drain_pending(doc).is_empty(),
            "a failure is not newly decoded"
        );
        assert!(!has_pending(doc), "but it is drained");
    }

    #[test]
    fn purge_pending_clears_has_pending() {
        let doc = rinch_core::dom::next_doc_key();
        push_pending(doc, "gone.png");
        assert!(has_pending(doc));
        purge_pending(doc);
        assert!(
            !has_pending(doc),
            "a torn-down document must stop asking its host to resolve"
        );
    }

    #[test]
    fn purge_pending_removes_only_own_entries() {
        let doc1 = rinch_core::dom::next_doc_key();
        let doc2 = rinch_core::dom::next_doc_key();

        push_pending(doc1, "doc1.png");
        push_pending(doc2, "doc2.png");
        purge_pending(doc1);
        assert_eq!(pending_count_for(doc1), 0, "doc1's entry must be purged");
        assert_eq!(
            pending_count_for(doc2),
            1,
            "doc2's entry must survive doc1's purge"
        );

        // Nothing left for doc1 to drain; doc2 still gets its image.
        let mut cache1 = ImageCache::new();
        let mut cache2 = ImageCache::new();
        assert!(cache1.drain_pending(doc1).is_empty());
        assert_eq!(cache2.drain_pending(doc2), vec!["doc2.png".to_string()]);
    }
}
