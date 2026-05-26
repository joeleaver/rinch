//! Cross-platform clipboard abstraction for rinch.
//!
//! This crate provides a unified clipboard API that works on both native platforms
//! (Windows, macOS, Linux) and WebAssembly (browser).
//!
//! # Platform Support
//!
//! - **Native**: Uses `arboard` for system clipboard access
//! - **WASM**: Uses the browser's Clipboard API via `web-sys`
//!
//! # Example
//!
//! ```ignore
//! use rinch_clipboard::{copy_text, paste_text, has_text};
//!
//! // Copy text to clipboard
//! copy_text("Hello, clipboard!").unwrap();
//!
//! // Check if clipboard has text
//! if has_text() {
//!     // Paste text from clipboard
//!     if let Ok(text) = paste_text() {
//!         println!("Clipboard: {}", text);
//!     }
//! }
//! ```
//!
//! # WASM Notes
//!
//! On WASM, clipboard operations are asynchronous due to browser security restrictions.
//! The synchronous API (`copy_text`, `paste_text`) uses a polling mechanism internally.
//! For better control in async contexts, use the `async` variants when available.

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
mod native;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_arch = "wasm32")]
mod web;

use std::borrow::Cow;

/// Clipboard error type.
#[derive(Debug)]
pub enum ClipboardError {
    /// Failed to access the clipboard.
    AccessFailed(String),
    /// The clipboard doesn't contain the expected content type.
    ContentTypeMismatch,
    /// Operation not supported on this platform.
    NotSupported,
    /// Clipboard is not available (e.g., no window focus in browser).
    NotAvailable,
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardError::AccessFailed(msg) => write!(f, "clipboard access failed: {}", msg),
            ClipboardError::ContentTypeMismatch => write!(f, "clipboard content type mismatch"),
            ClipboardError::NotSupported => write!(f, "clipboard operation not supported"),
            ClipboardError::NotAvailable => write!(f, "clipboard not available"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// Result type for clipboard operations.
pub type ClipboardResult<T> = Result<T, ClipboardError>;

/// Image data for clipboard operations.
///
/// The bytes are in RGBA format (4 bytes per pixel).
#[derive(Debug, Clone)]
pub struct ImageData<'a> {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// RGBA pixel data.
    pub bytes: Cow<'a, [u8]>,
}

impl<'a> ImageData<'a> {
    /// Create new image data.
    pub fn new(width: usize, height: usize, bytes: impl Into<Cow<'a, [u8]>>) -> Self {
        Self {
            width,
            height,
            bytes: bytes.into(),
        }
    }

    /// Convert to owned data.
    pub fn into_owned(self) -> ImageData<'static> {
        ImageData {
            width: self.width,
            height: self.height,
            bytes: Cow::Owned(self.bytes.into_owned()),
        }
    }
}

// Re-export platform-specific implementations
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub use native::*;

#[cfg(target_os = "android")]
pub use android::*;

#[cfg(target_arch = "wasm32")]
pub use web::*;
