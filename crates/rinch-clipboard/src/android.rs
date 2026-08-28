//! Android clipboard implementation via rinch-android JNI bridge.

use super::{ClipboardError, ClipboardResult, ImageData};

pub fn copy_text(text: impl AsRef<str>) -> ClipboardResult<()> {
    rinch_android::clipboard::copy_text(text.as_ref()).map_err(|e| ClipboardError::AccessFailed(e))
}

pub fn paste_text() -> ClipboardResult<String> {
    rinch_android::clipboard::paste_text().map_err(|e| ClipboardError::AccessFailed(e))
}

pub fn has_text() -> bool {
    rinch_android::clipboard::has_text()
}

pub fn clear() -> ClipboardResult<()> {
    copy_text("")
}

pub fn copy_image(_image: ImageData) -> ClipboardResult<()> {
    Err(ClipboardError::NotSupported)
}

pub fn paste_image() -> ClipboardResult<ImageData<'static>> {
    Err(ClipboardError::NotSupported)
}

pub fn has_image() -> bool {
    false
}

pub fn copy_html(_html: impl AsRef<str>, _alt_text: Option<&str>) -> ClipboardResult<()> {
    Err(ClipboardError::NotSupported)
}

pub fn paste_html() -> ClipboardResult<String> {
    Err(ClipboardError::NotSupported)
}

pub fn has_html() -> bool {
    false
}

// ── Timed / callback API ──────────────────────────────────────────────────────
//
// Android's ClipboardManager is an in-process binder call, not a request to
// another app that could hang the way an X11 selection owner can (#149), so
// there is nothing here to move off the caller's thread: every variant is a
// passthrough. They exist so one piece of app code compiles on every target.
//
// The callback therefore runs **on the calling thread**, synchronously, before
// the `*_async` call returns.

use std::time::Duration;

use super::RichPaste;

pub fn copy_text_async(text: impl AsRef<str>) {
    let _ = copy_text(text);
}

pub fn paste_text_timeout(_timeout: Duration) -> ClipboardResult<String> {
    paste_text()
}

pub fn paste_text_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    on_done(paste_text());
}

pub fn paste_image_timeout(_timeout: Duration) -> ClipboardResult<ImageData<'static>> {
    paste_image()
}

pub fn paste_image_async(
    on_done: impl FnOnce(ClipboardResult<ImageData<'static>>) + Send + 'static,
) {
    on_done(paste_image());
}

pub fn copy_html_async(html: impl AsRef<str>, alt_text: Option<&str>) {
    let _ = copy_html(html, alt_text);
}

pub fn paste_html_timeout(_timeout: Duration) -> ClipboardResult<String> {
    paste_html()
}

pub fn paste_html_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    on_done(paste_html());
}

/// The richest content available — text only, since Android's bridge exposes no
/// `text/html` or bitmap yet.
pub fn paste_rich() -> ClipboardResult<RichPaste> {
    match paste_text() {
        Ok(text) if !text.is_empty() => Ok(RichPaste::Text(text)),
        Ok(_) => Err(ClipboardError::ContentTypeMismatch),
        Err(e) => Err(e),
    }
}

pub fn paste_rich_timeout(_timeout: Duration) -> ClipboardResult<RichPaste> {
    paste_rich()
}

pub fn paste_rich_async(on_done: impl FnOnce(ClipboardResult<RichPaste>) + Send + 'static) {
    on_done(paste_rich());
}
