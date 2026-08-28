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
//! # Reading without stalling the UI (issue #149)
//!
//! A clipboard *read* is a request to another process. On X11 the owner may be
//! hung, and arboard waits up to **4 seconds** for it; on Wayland and on the web
//! the read is likewise not instantaneous. Calling [`paste_text`] from an event
//! handler therefore blocks the UI thread for as long as the owner takes.
//!
//! Every platform backend here exposes the same three shapes:
//!
//! | Shape | Blocks the caller? | Use for |
//! |---|---|---|
//! | [`paste_text`] | yes, indefinitely | scripts, worker threads, back-compat |
//! | [`paste_text_timeout`] | yes, bounded | an interactive path that can accept a bounded hiccup |
//! | [`paste_text_async`] | no | an interactive path that must stay responsive |
//!
//! On native, all three are served by **one dedicated clipboard worker thread**
//! that owns the `arboard::Clipboard`. That is what makes the timeout useful: a
//! caller that gives up does not cancel the request, it merely stops waiting for
//! it, so the abandoned read completes on the worker and the requests queued
//! behind it are not wedged behind a lock the abandoning caller still holds.
//!
//! [`paste_rich`] / [`paste_rich_async`] resolve `text/html` → bitmap →
//! `text/plain` in a **single** worker pass, so a rich-paste consumer never
//! stacks three worst-case stalls.
//!
//! ## Which thread does an async callback run on?
//!
//! **Not necessarily the UI thread.** On native the callback runs on the
//! clipboard worker thread — hence the `Send` bound — and on the web it runs on
//! the single browser thread. rinch UI state is thread-local, so a native
//! callback must hop back before touching it:
//!
//! ```ignore
//! let id = rinch_core::park_main_callback::<ClipboardResult<String>>(move |r| {
//!     // main thread: free to touch Signals, an EditorHandle, the DOM
//! });
//! rinch_clipboard::paste_text_async(move |r| {
//!     rinch_core::run_on_main_thread(move || rinch_core::resume_main_callback(id, r));
//! });
//! ```
//!
//! For the same reason, a blocking call (`paste_text`, `paste_text_timeout`)
//! made *from inside* an async callback would wait on the worker that is running
//! it. That is detected and reported as an error rather than deadlocking; use the
//! `*_async` variants, which just queue.
//!
//! # WASM Notes
//!
//! The browser cannot be read synchronously at all, so on WASM [`paste_text`]
//! answers from a buffer filled by [`copy_text`] and by the host page's `paste`
//! ClipboardEvent (rinch-web wires that up — issue #150), while
//! [`paste_text_async`] additionally tries `navigator.clipboard.readText()`.

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
mod native;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_arch = "wasm32")]
mod web;

use std::borrow::Cow;

/// Clipboard error type.
///
/// `#[non_exhaustive]`: reading the system clipboard can fail in
/// platform-specific ways we may need to name later, so downstream `match`es
/// must carry a wildcard arm rather than break on a new variant.
#[derive(Debug)]
#[non_exhaustive]
pub enum ClipboardError {
    /// Failed to access the clipboard.
    AccessFailed(String),
    /// The clipboard doesn't contain the expected content type.
    ContentTypeMismatch,
    /// Operation not supported on this platform.
    NotSupported,
    /// Clipboard is not available (e.g., no window focus in browser).
    NotAvailable,
    /// A `*_timeout` read gave up before the clipboard owner answered.
    ///
    /// The request is **not** cancelled: it finishes on the clipboard worker,
    /// so it never wedges the requests queued behind it (see the module docs).
    TimedOut,
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardError::AccessFailed(msg) => write!(f, "clipboard access failed: {}", msg),
            ClipboardError::ContentTypeMismatch => write!(f, "clipboard content type mismatch"),
            ClipboardError::NotSupported => write!(f, "clipboard operation not supported"),
            ClipboardError::NotAvailable => write!(f, "clipboard not available"),
            ClipboardError::TimedOut => write!(f, "clipboard read timed out"),
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

/// The richest content available on the clipboard, resolved in **one** pass.
///
/// Returned by [`paste_rich`] / [`paste_rich_async`], which probe `text/html` →
/// bitmap → `text/plain` in a single trip to the clipboard rather than making the
/// caller chain three independent reads (each of which can independently stall
/// against a hung selection owner — the bug in issue #149).
#[derive(Debug)]
pub enum RichPaste {
    /// `text/html` — rich content, structure and marks preserved.
    Html(String),
    /// A raw RGBA bitmap with no HTML wrapper (a screenshot, a "copy image").
    Image(ImageData<'static>),
    /// `text/plain`.
    Text(String),
}

impl RichPaste {
    /// The plain-text payload, for a caller that only wants text
    /// ("paste and match style"). `None` for an image.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            RichPaste::Html(_) => None,
            RichPaste::Image(_) => None,
            RichPaste::Text(t) => Some(t),
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
