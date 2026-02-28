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
    pub src: String,
    pub result: Result<DecodedImage, String>,
}

/// Thread-safe queue for completed image loads from background threads.
///
/// Background threads push into this; the main thread drains it during layout.
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

    /// Drain the pending images queue and insert into this cache.
    ///
    /// Returns the list of source strings that were newly decoded (for re-layout).
    pub fn drain_pending(&mut self) -> Vec<String> {
        let pending: Vec<PendingImage> = PENDING_IMAGES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
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
pub fn request_image_load(src: String, loader: Arc<dyn ImageLoader>) {
    std::thread::spawn(move || {
        let result = loader.load(&src);
        let pending = match result {
            ImageLoadResult::Loaded(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    PendingImage {
                        src,
                        result: Ok(DecodedImage {
                            data: rgba.into_raw(),
                            width: w,
                            height: h,
                        }),
                    }
                }
                Err(e) => PendingImage {
                    src,
                    result: Err(format!("Failed to decode image: {}", e)),
                },
            },
            ImageLoadResult::Failed(e) => PendingImage {
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
