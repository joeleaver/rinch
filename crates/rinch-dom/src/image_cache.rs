//! Image cache and loading pipeline for rinch-dom.
//!
//! Manages decoded images for `<img>` elements and `background-image` CSS.
//! Images are loaded asynchronously on background threads and decoded into
//! RGBA8 pixel data suitable for Vello rendering.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rinch_core::image::{ImageLoadResult, ImageLoader};

/// A decoded image ready for Vello rendering.
pub struct DecodedImage {
    /// Raw RGBA8 pixel data.
    pub data: Vec<u8>,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
}

/// State of an image in the cache.
enum ImageState {
    /// Image is currently being loaded/decoded on a background thread.
    Loading,
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

/// Cache of loaded images, keyed by source string (file path or URL).
pub struct ImageCache {
    entries: HashMap<String, ImageState>,
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
        self.entries.insert(src, ImageState::Loading);
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
            match item.result {
                Ok(img) => {
                    newly_decoded.push(item.src.clone());
                    self.entries.insert(item.src, ImageState::Decoded(img));
                }
                Err(e) => {
                    tracing::warn!("Image load failed for {}: {}", item.src, e);
                    self.entries.insert(item.src, ImageState::Failed(e));
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
        let result = loader.load(&src);
        let pending = match result {
            ImageLoadResult::Loaded(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    PendingImage {
                        doc_key,
                        src,
                        result: Ok(DecodedImage {
                            data: rgba.into_raw(),
                            width: w,
                            height: h,
                        }),
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
    });
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
                result: Ok(DecodedImage {
                    data: vec![0; 4],
                    width: 1,
                    height: 1,
                }),
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
