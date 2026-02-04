//! WASM clipboard implementation using the browser Clipboard API.
//!
//! The browser Clipboard API is inherently async. This module provides
//! synchronous wrappers that use an internal buffer for copy/paste within
//! the app, and also attempt to access the system clipboard when possible.

use crate::{ClipboardError, ClipboardResult, ImageData};
use std::cell::RefCell;

// Internal buffer for synchronous clipboard operations.
// On the web, we can't synchronously read the system clipboard,
// so we maintain a local buffer for copy/paste within the app.
thread_local! {
    static CLIPBOARD_BUFFER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Copy text to the clipboard.
///
/// On WASM, this stores the text in a local buffer and also attempts
/// to write to the system clipboard asynchronously.
pub fn copy_text(text: impl AsRef<str>) -> ClipboardResult<()> {
    let text = text.as_ref().to_string();

    // Always store locally for synchronous paste_text() to work
    CLIPBOARD_BUFFER.with(|buf| {
        *buf.borrow_mut() = Some(text.clone());
    });

    // Also try to write to the system clipboard (fire-and-forget)
    if let Some(window) = web_sys::window() {
        if let Ok(navigator) = js_sys::Reflect::get(&window, &"navigator".into()) {
            if let Ok(clipboard) = js_sys::Reflect::get(&navigator, &"clipboard".into()) {
                if !clipboard.is_undefined() {
                    let clipboard: web_sys::Clipboard = clipboard.into();
                    let _ = clipboard.write_text(&text);
                }
            }
        }
    }

    Ok(())
}

/// Paste text from the clipboard.
///
/// On WASM, this reads from the local buffer. The browser's async Clipboard API
/// cannot be accessed synchronously; for cross-tab paste, the host page should
/// intercept paste events and forward text via `EditCommand::Paste`.
pub fn paste_text() -> ClipboardResult<String> {
    CLIPBOARD_BUFFER.with(|buf| {
        buf.borrow()
            .clone()
            .ok_or(ClipboardError::ContentTypeMismatch)
    })
}

/// Check if the clipboard contains text.
pub fn has_text() -> bool {
    CLIPBOARD_BUFFER.with(|buf| buf.borrow().is_some())
}

/// Clear the clipboard contents.
pub fn clear() -> ClipboardResult<()> {
    CLIPBOARD_BUFFER.with(|buf| {
        *buf.borrow_mut() = None;
    });
    Ok(())
}

/// Copy an image to the clipboard.
///
/// Not currently supported on WASM.
pub fn copy_image(_image: ImageData) -> ClipboardResult<()> {
    Err(ClipboardError::NotSupported)
}

/// Paste an image from the clipboard.
///
/// Not currently supported on WASM.
pub fn paste_image() -> ClipboardResult<ImageData<'static>> {
    Err(ClipboardError::NotSupported)
}

/// Check if the clipboard contains an image.
pub fn has_image() -> bool {
    false
}
