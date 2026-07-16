//! Network-capable image loader (optional, requires `image-network` feature).
//!
//! HTTP(S) image loads go through [`rinch_http::fetch_blocking`] rather than a
//! private ureq call, so they share the app's one HTTP agent — its cookie jar,
//! connection pool and timeouts. Concretely, an image behind the same
//! cookie-session the app logged in with now loads authenticated, instead of
//! going out on a fresh, cookie-less agent. This runs on the image-decode worker
//! thread (`ImageLoader::load` is called off the main thread), which is exactly
//! what the blocking entry point is for.

// Network image loading is a native concern: image loads run on a background
// thread via `std::thread::spawn` (a no-op on wasm, where the browser fetches
// `<img>` itself), and the shared blocking client lives on the native target.
#[cfg(all(feature = "image-network", not(target_arch = "wasm32")))]
use rinch_core::image::{ImageLoadResult, ImageLoader};

/// Image loader that supports both local files and HTTP(S) URLs.
///
/// Enabled with the `image-network` feature flag.
/// Falls back to filesystem loading for non-URL paths.
#[cfg(all(feature = "image-network", not(target_arch = "wasm32")))]
pub struct NetworkImageLoader;

#[cfg(all(feature = "image-network", not(target_arch = "wasm32")))]
impl ImageLoader for NetworkImageLoader {
    fn load(&self, src: &str) -> ImageLoadResult {
        if src.starts_with("http://") || src.starts_with("https://") {
            // Share the app's HTTP agent (cookie jar + connection pool) instead of
            // a fresh per-call agent, so authenticated image URLs carry the session.
            match rinch_http::fetch_blocking(rinch_http::Request::get(src)) {
                Ok(resp) if resp.ok() => ImageLoadResult::Loaded(resp.body),
                Ok(resp) => {
                    ImageLoadResult::Failed(format!("HTTP {} loading image {}", resp.status, src))
                }
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
