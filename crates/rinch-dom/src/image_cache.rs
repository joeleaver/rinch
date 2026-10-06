//! Image cache and loading pipeline for rinch-dom.
//!
//! Manages decoded images for `<img>` elements and `background-image` CSS.
//! Images are loaded asynchronously on background threads and decoded into
//! RGBA8 pixel data suitable for Vello rendering.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rinch_core::image::{ImageLoadResult, ImageLoader};

/// A decoded image ready to paint.
pub struct DecodedImage {
    /// Raw RGBA8 pixel data, straight (not premultiplied) alpha.
    pub data: Vec<u8>,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// The same pixels premultiplied, made on the first software paint and
    /// kept (the software painter needs premultiplied pixels, and making them
    /// is a pass over the whole image). `None` inside means every pixel is
    /// opaque, so the premultiplied pixels *are* `data` and no copy is kept.
    /// Never filled on the GPU path, which takes straight alpha.
    premultiplied: std::sync::OnceLock<Option<Vec<u8>>>,
    /// Process-unique, never reused: what a cache of pixels derived from this
    /// image keys on (a data pointer can be reused by the next image).
    id: u64,
}

impl DecodedImage {
    /// A decoded image over straight-alpha RGBA8 `data`.
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            data,
            width,
            height,
            premultiplied: std::sync::OnceLock::new(),
            id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    /// This image's process-unique identity.
    pub fn id(&self) -> u64 {
        self.id
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
    /// Image has been decoded and is ready to paint.
    Decoded(DecodedImage),
    /// Image loading or decoding failed.
    #[allow(dead_code)]
    Failed(String),
}

/// A completed image load result, pending insertion into the main cache.
pub struct PendingImage {
    /// Identity of the requesting document ([`RinchDocument::doc_key`]) — the
    /// queue is process-global, so entries must be tagged so each document
    /// drains only its own decodes (issue #137).
    pub doc_key: u64,
    pub src: String,
    pub result: Result<DecodedImage, String>,
}

/// Thread-safe queue for completed image loads from background threads.
///
/// Background threads push into this; the main thread drains it during layout.
/// Entries are tagged with the requesting document's `doc_key` — each
/// document's [`ImageCache::drain_pending`] removes only its own entries.
static PENDING_IMAGES: Mutex<Vec<PendingImage>> = Mutex::new(Vec::new());

/// Sources [`reload_image`] named, one entry per live document, waiting for
/// that document's next [`RinchDocument::drain_pending_images`](crate::RinchDocument::drain_pending_images).
static PENDING_RELOADS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

/// The `doc_key` of every live [`RinchDocument`](crate::RinchDocument), so a
/// reload asked for from any thread reaches each document's cache and no
/// entry is ever queued for a document that can no longer drain it.
static LIVE_DOCUMENTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// Cache of loaded images, keyed by source string (file path or URL).
pub struct ImageCache {
    entries: HashMap<String, ImageState>,
    /// Sources whose in-flight answer was discarded by [`Self::drain_pending`]
    /// because a reload was asked for meanwhile: the document starts them again.
    retries: Vec<String>,
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
            retries: Vec::new(),
        }
    }

    /// Get a decoded image if available.
    pub fn get(&self, src: &str) -> Option<&DecodedImage> {
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
        self.entries
            .insert(src, ImageState::Loading { reload: false });
    }

    /// Whether loading or decoding `src` failed (and nothing has asked for it
    /// again since).
    pub fn is_failed(&self, src: &str) -> bool {
        matches!(self.entries.get(src), Some(ImageState::Failed(_)))
    }

    /// Forget what is known about `src` so that it is asked for again: the
    /// cache half of [`reload_image`]. Returns whether the caller should start
    /// a load for it now.
    ///
    /// - A **failed** source goes back to loading: `true`.
    /// - A **decoded** one keeps its pixels on screen until the new answer
    ///   replaces them (no flash of an empty box): `true`. If the new load
    ///   fails, the pixels it had are kept.
    /// - One whose load is **in flight** is marked, so the answer on its way is
    ///   discarded and the source asked for once more ([`Self::take_retries`]):
    ///   `false`, nothing to start yet.
    /// - A source nothing has asked for has nothing to forget: `false`. It is
    ///   loaded fresh when an element first names it.
    pub fn begin_reload(&mut self, src: &str) -> bool {
        match self.entries.get_mut(src) {
            Some(state @ ImageState::Failed(_)) => {
                *state = ImageState::Loading { reload: false };
                true
            }
            Some(ImageState::Decoded(_)) => true,
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

    /// Insert a decoded image into the cache.
    pub fn insert_decoded(&mut self, src: String, image: DecodedImage) {
        self.entries.insert(src, ImageState::Decoded(image));
    }

    /// Mark a source as failed.
    pub fn mark_failed(&mut self, src: String, error: String) {
        self.entries.insert(src, ImageState::Failed(error));
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
            match item.result {
                Ok(img) => {
                    newly_decoded.push(item.src.clone());
                    self.entries.insert(item.src, ImageState::Decoded(img));
                }
                Err(e) => {
                    tracing::warn!("Image load failed for {}: {}", item.src, e);
                    // A reload of a picture that is on screen keeps it there.
                    if !matches!(self.entries.get(&item.src), Some(ImageState::Decoded(_))) {
                        self.entries.insert(item.src, ImageState::Failed(e));
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

/// Spawn a background thread to load and decode an image.
///
/// When complete, the result is pushed to the global pending queue.
/// Call [`ImageCache::drain_pending()`] from the main thread to collect results.
///
/// On `wasm32-unknown-unknown` there are no OS threads (`std::thread::spawn` would
/// panic), so file/remote loads are a no-op there. Synchronous `data:` URIs never
/// reach this path — they are decoded inline via [`decode_data_uri`] — so embedded
/// (base64) images still render on the web. (issue #97)
#[cfg(target_arch = "wasm32")]
pub fn request_image_load(_doc_key: u64, _src: String, _loader: Arc<dyn ImageLoader>) {}

/// Spawn a background thread to load and decode an image (native).
///
/// The result lands in the pending queue tagged with `doc_key` so only the
/// requesting document's [`ImageCache::drain_pending`] picks it up.
#[cfg(not(target_arch = "wasm32"))]
pub fn request_image_load(doc_key: u64, src: String, loader: Arc<dyn ImageLoader>) {
    std::thread::spawn(move || {
        // A scheme the app installed a loader for is the app's to answer;
        // everything else is the document's loader's.
        let loader = rinch_core::image::image_scheme_loader(&src).unwrap_or(loader);
        let result = loader.load(&src);
        let pending = match result {
            ImageLoadResult::Loaded(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    PendingImage {
                        doc_key,
                        src,
                        result: Ok(DecodedImage::new(rgba.into_raw(), w, h)),
                    }
                }
                Err(e) => PendingImage {
                    doc_key,
                    src,
                    result: Err(format!("Failed to decode image: {}", e)),
                },
            },
            ImageLoadResult::Failed(e) => PendingImage {
                doc_key,
                src,
                result: Err(e),
            },
        };
        PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(pending);

        // Wake the main thread. The desktop event loop runs on
        // `ControlFlow::Wait`, so without this the decode sits in the queue
        // until some other input happens to arrive — an `<img src>` set while
        // the app is otherwise idle stays 0x0 and unpainted, in practice
        // forever. Dispatching an empty callback is the cheapest thing that
        // goes through the platform wake path.
        //
        // The wake is *promptness only*, never the correctness guarantee, and
        // must not be treated as one: hosts install their own dispatcher, and
        // an embedded one deliberately wakes nothing (its game loop is already
        // turning); the desktop one coalesces, so a wake can be swallowed when
        // another closure is already queued. What actually guarantees the drain
        // is [`has_pending`], which every host's "is there anything to do?"
        // gate consults.
        rinch_core::run_on_main_thread(|| {});
    });
}

/// Whether any decode is queued for this document.
///
/// A completed image load dirties no DOM node — the decoding thread has no idea
/// which nodes reference the source, and [`ImageCache::drain_pending`] is what
/// works that out — so a frame loop that short-circuits on "nothing is dirty"
/// skips the drain and the image never gets its intrinsic size. Anything that
/// gates layout on dirtiness has to ask this too.
pub fn has_pending(doc_key: u64) -> bool {
    PENDING_IMAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|item| item.doc_key == doc_key)
        || PENDING_RELOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(key, _)| *key == doc_key)
}

/// Ask every document to load `src` again: the way a picture that failed to
/// load, or whose bytes have changed, gets another go without a restart.
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
/// old pixels until the new ones land; one whose load is still in flight has
/// that answer discarded and is asked for once more, so a "not yet" that was
/// already on its way cannot beat the reload; a source no element has asked
/// for is left alone (it loads fresh when one does). `data:` sources never
/// change and are ignored.
pub fn reload_image(src: &str) {
    if src.is_empty() || src.starts_with("data:") {
        return;
    }
    {
        let live = LIVE_DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
        let mut reloads = PENDING_RELOADS.lock().unwrap_or_else(|e| e.into_inner());
        for &doc_key in live.iter() {
            if !reloads.iter().any(|(key, s)| *key == doc_key && s == src) {
                reloads.push((doc_key, src.to_string()));
            }
        }
    }
    // Promptness only, as for a finished decode: `has_pending` is what makes
    // every host's frame gate come round to the drain.
    rinch_core::run_on_main_thread(|| {});
}

/// Take the sources [`reload_image`] queued for this document.
pub(crate) fn take_pending_reloads(doc_key: u64) -> Vec<String> {
    PENDING_RELOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extract_if(.., |(key, _)| *key == doc_key)
        .map(|(_, src)| src)
        .collect()
}

/// Note a document as live, so [`reload_image`] reaches it. Undone by
/// [`purge_pending`] when the document drops.
pub(crate) fn register_document(doc_key: u64) {
    LIVE_DOCUMENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(doc_key);
}

/// Remove all queued entries for a document that is being torn down.
///
/// Without this, decodes that land after a document is dropped would strand in
/// the process-global queue forever (nothing drains a dead doc_key). Called
/// from `RinchDocument::drop` (issue #137).
pub fn purge_pending(doc_key: u64) {
    PENDING_IMAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|item| item.doc_key != doc_key);
    LIVE_DOCUMENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|key| *key != doc_key);
    PENDING_RELOADS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(key, _)| *key != doc_key);
}

/// Decode a `data:` URI into raw bytes.
///
/// Supports `data:[<mediatype>];base64,<data>` format.
/// Returns `None` if the URI is malformed or not base64-encoded.
pub fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let comma = src.find(',')?;
    let header = &src[..comma];
    let data = &src[comma + 1..];
    if header.contains(";base64") {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(data).ok()
    } else {
        None
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
                result: Ok(DecodedImage::new(vec![0; 4], 1, 1)),
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
