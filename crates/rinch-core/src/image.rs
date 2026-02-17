//! Image loading abstraction for rinch.
//!
//! Defines the [`ImageLoader`] trait which backends use to load images
//! from various sources (local files, network, etc.).

/// Result of an image load operation.
pub enum ImageLoadResult {
    /// Successfully loaded raw encoded bytes (PNG/JPEG/GIF/WebP).
    Loaded(Vec<u8>),
    /// Loading failed with an error message.
    Failed(String),
}

/// Trait for loading image data from a source string (file path or URL).
///
/// Implementations run on a background thread via [`std::thread::spawn`],
/// so blocking I/O is acceptable.
pub trait ImageLoader: Send + Sync + 'static {
    /// Load image bytes from the given source.
    ///
    /// `src` is typically a file path or URL, depending on the loader.
    fn load(&self, src: &str) -> ImageLoadResult;
}
