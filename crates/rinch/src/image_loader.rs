//! Network-capable image loader (optional, requires `image-network` feature).

#[cfg(feature = "image-network")]
use rinch_core::image::{ImageLoadResult, ImageLoader};

/// Image loader that supports both local files and HTTP(S) URLs.
///
/// Enabled with the `image-network` feature flag.
/// Falls back to filesystem loading for non-URL paths.
#[cfg(feature = "image-network")]
pub struct NetworkImageLoader;

#[cfg(feature = "image-network")]
impl ImageLoader for NetworkImageLoader {
    fn load(&self, src: &str) -> ImageLoadResult {
        if src.starts_with("http://") || src.starts_with("https://") {
            match ureq::get(src).call() {
                Ok(resp) => match resp.into_body().read_to_vec() {
                    Ok(bytes) => ImageLoadResult::Loaded(bytes),
                    Err(e) => ImageLoadResult::Failed(format!(
                        "Failed to read response body for {}: {}",
                        src, e
                    )),
                },
                Err(e) => {
                    ImageLoadResult::Failed(format!("HTTP request failed for {}: {}", src, e))
                }
            }
        } else {
            // Fall through to file loading
            match std::fs::read(src) {
                Ok(bytes) => ImageLoadResult::Loaded(bytes),
                Err(e) => ImageLoadResult::Failed(format!("Failed to read {}: {}", src, e)),
            }
        }
    }
}
