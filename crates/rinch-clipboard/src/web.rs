//! WASM clipboard implementation using the browser Clipboard API.
//!
//! # Where the content comes from (issue #150)
//!
//! The browser has no synchronous system-clipboard read at all, so the blocking
//! API answers from two thread-local buffers. Three things fill them:
//!
//! 1. [`copy_text`] / [`copy_html`] — what this app copied.
//! 2. [`fill_buffers_from_event`] — what the **host page's `paste`
//!    ClipboardEvent** carried. `ClipboardEvent.clipboardData` is the only
//!    synchronous channel to content copied in *another* app or tab, and rinch-web
//!    installs the document-level listener that feeds it here. Without that, an
//!    app on the web could only ever paste its own copies (issue #150).
//! 3. [`paste_text_async`] — `navigator.clipboard.readText()`, a real promise,
//!    which also refreshes the buffer on the way through.
//!
//! The async API is therefore the *native* shape here rather than a wrapper: it
//! is how the platform actually works, and it is why #149 and #150 share one API
//! (a callback-based read serves an X11 stall and a browser promise equally).

use crate::{ClipboardError, ClipboardResult, ImageData, RichPaste};
use std::cell::RefCell;
use std::time::Duration;

// Internal buffers for synchronous clipboard operations.
// On the web, we can't synchronously read the system clipboard,
// so we maintain local buffers for copy/paste within the app.
thread_local! {
    static CLIPBOARD_BUFFER: RefCell<Option<String>> = const { RefCell::new(None) };
    static HTML_CLIPBOARD_BUFFER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The browser's `navigator.clipboard`, if this context has one (it is absent on
/// insecure origins and in some embedded webviews).
fn navigator_clipboard() -> Option<web_sys::Clipboard> {
    let window = web_sys::window()?;
    let navigator = js_sys::Reflect::get(&window, &"navigator".into()).ok()?;
    let clipboard = js_sys::Reflect::get(&navigator, &"clipboard".into()).ok()?;
    if clipboard.is_undefined() || clipboard.is_null() {
        return None;
    }
    Some(clipboard.into())
}

/// Fill the clipboard buffers from a host-page `paste`/`copy` ClipboardEvent.
///
/// This is the bridge that lets content copied **outside the app** reach
/// [`paste_text`] / [`paste_html`] on the web (issue #150): a `ClipboardEvent`
/// carries `text/plain` and `text/html` synchronously, which no other browser API
/// does. rinch-web calls this from its document-level listener before running app
/// paste logic, so a handler that then calls `paste_text()` sees the fresh content.
///
/// `None` leaves that buffer alone — a `paste` event with no `text/html` must not
/// erase HTML the app itself copied a moment ago.
pub fn fill_buffers_from_event(text: Option<String>, html: Option<String>) {
    if let Some(text) = text {
        CLIPBOARD_BUFFER.with(|buf| {
            *buf.borrow_mut() = Some(text);
        });
    }
    if let Some(html) = html {
        HTML_CLIPBOARD_BUFFER.with(|buf| {
            *buf.borrow_mut() = Some(html);
        });
    }
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
    if let Some(clipboard) = navigator_clipboard() {
        let _ = clipboard.write_text(&text);
    }

    Ok(())
}

/// [`copy_text`] without waiting — identical here, since the browser write is
/// already fire-and-forget. Present so the async API is the same on every target.
pub fn copy_text_async(text: impl AsRef<str>) {
    let _ = copy_text(text);
}

/// Paste text from the clipboard.
///
/// Reads the buffer filled by [`copy_text`] and by the host page's `paste` event
/// (see [`fill_buffers_from_event`]). The browser's async Clipboard API cannot be
/// reached synchronously — use [`paste_text_async`] to also consult it.
pub fn paste_text() -> ClipboardResult<String> {
    CLIPBOARD_BUFFER.with(|buf| {
        buf.borrow()
            .clone()
            .ok_or(ClipboardError::ContentTypeMismatch)
    })
}

/// [`paste_text`]; the timeout is unused because the read cannot block.
pub fn paste_text_timeout(_timeout: Duration) -> ClipboardResult<String> {
    paste_text()
}

/// Read text asynchronously, consulting the real system clipboard.
///
/// Tries `navigator.clipboard.readText()` — which needs a secure context and, in
/// most browsers, a user gesture or the `clipboard-read` permission — and falls
/// back to the buffer when it is unavailable or rejects. A successful read also
/// refreshes the buffer, so a later synchronous [`paste_text`] sees it.
///
/// `on_done` runs on the browser's single thread, i.e. the UI thread.
pub fn paste_text_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    let Some(clipboard) = navigator_clipboard() else {
        on_done(paste_text());
        return;
    };
    let promise = clipboard.read_text();
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(promise).await {
            Ok(value) => match value.as_string() {
                Some(text) => {
                    fill_buffers_from_event(Some(text.clone()), None);
                    on_done(Ok(text));
                }
                // A resolved read that isn't a string shouldn't happen; treat it
                // as "nothing readable" rather than inventing content.
                None => on_done(paste_text()),
            },
            // Denied permission, an insecure origin, no user gesture: the buffer
            // is still the app's best answer.
            Err(_) => on_done(paste_text()),
        }
    });
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
    HTML_CLIPBOARD_BUFFER.with(|buf| {
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
/// Not currently supported on WASM. The browser delivers pasted bitmaps as a
/// `File` on the `paste` event instead; rinch-web's editor reads them there.
pub fn paste_image() -> ClipboardResult<ImageData<'static>> {
    Err(ClipboardError::NotSupported)
}

/// [`paste_image`]; the timeout is unused because the read cannot block.
pub fn paste_image_timeout(_timeout: Duration) -> ClipboardResult<ImageData<'static>> {
    paste_image()
}

/// [`paste_image`] delivered by callback. Always reports `NotSupported`.
pub fn paste_image_async(
    on_done: impl FnOnce(ClipboardResult<ImageData<'static>>) + Send + 'static,
) {
    on_done(paste_image());
}

/// Check if the clipboard contains an image.
pub fn has_image() -> bool {
    false
}

/// Copy HTML to the clipboard with a plain-text fallback.
///
/// On WASM, stores both HTML and plain text in local buffers.
pub fn copy_html(html: impl AsRef<str>, alt_text: Option<&str>) -> ClipboardResult<()> {
    let html = html.as_ref().to_string();

    // Store HTML locally
    HTML_CLIPBOARD_BUFFER.with(|buf| {
        *buf.borrow_mut() = Some(html.clone());
    });

    // Also store plain text version
    if let Some(text) = alt_text {
        CLIPBOARD_BUFFER.with(|buf| {
            *buf.borrow_mut() = Some(text.to_string());
        });
    }

    // Try to write to system clipboard (fire-and-forget)
    // Note: browser Clipboard API doesn't have a simple setHtml, but we can
    // use ClipboardItem with text/html blob for modern browsers
    if let Some(clipboard) = navigator_clipboard() {
        // Fall back to writing plain text for broad compatibility
        if let Some(text) = alt_text {
            let _ = clipboard.write_text(text);
        }
    }

    Ok(())
}

/// [`copy_html`] without waiting — identical here (the browser write is already
/// fire-and-forget). Present so the async API is the same on every target.
pub fn copy_html_async(html: impl AsRef<str>, alt_text: Option<&str>) {
    let _ = copy_html(html, alt_text);
}

/// Paste HTML from the clipboard.
///
/// Reads the buffer filled by [`copy_html`] and by the host page's `paste` event
/// (see [`fill_buffers_from_event`]).
pub fn paste_html() -> ClipboardResult<String> {
    HTML_CLIPBOARD_BUFFER.with(|buf| {
        buf.borrow()
            .clone()
            .ok_or(ClipboardError::ContentTypeMismatch)
    })
}

/// [`paste_html`]; the timeout is unused because the read cannot block.
pub fn paste_html_timeout(_timeout: Duration) -> ClipboardResult<String> {
    paste_html()
}

/// [`paste_html`] delivered by callback (on the browser's UI thread).
///
/// Answers from the buffer: `navigator.clipboard.read()` can carry `text/html`,
/// but on the web the `paste` event already delivers it synchronously and with
/// no permission prompt, so the buffer is both fresher and cheaper.
pub fn paste_html_async(on_done: impl FnOnce(ClipboardResult<String>) + Send + 'static) {
    on_done(paste_html());
}

/// Check if the clipboard contains HTML.
pub fn has_html() -> bool {
    HTML_CLIPBOARD_BUFFER.with(|buf| buf.borrow().is_some())
}

/// The richest buffered content: `text/html`, else `text/plain`.
pub fn paste_rich() -> ClipboardResult<RichPaste> {
    if let Ok(html) = paste_html() {
        if !html.trim().is_empty() {
            return Ok(RichPaste::Html(html));
        }
    }
    match paste_text() {
        Ok(text) if !text.is_empty() => Ok(RichPaste::Text(text)),
        Ok(_) => Err(ClipboardError::ContentTypeMismatch),
        Err(e) => Err(e),
    }
}

/// [`paste_rich`]; the timeout is unused because the read cannot block.
pub fn paste_rich_timeout(_timeout: Duration) -> ClipboardResult<RichPaste> {
    paste_rich()
}

/// [`paste_rich`] delivered by callback.
///
/// HTML comes from the buffer; when there is none, the plain text is read through
/// [`paste_text_async`] so the system clipboard is still consulted.
pub fn paste_rich_async(on_done: impl FnOnce(ClipboardResult<RichPaste>) + Send + 'static) {
    if let Ok(html) = paste_html() {
        if !html.trim().is_empty() {
            on_done(Ok(RichPaste::Html(html)));
            return;
        }
    }
    paste_text_async(move |result| {
        on_done(match result {
            Ok(text) if !text.is_empty() => Ok(RichPaste::Text(text)),
            Ok(_) => Err(ClipboardError::ContentTypeMismatch),
            Err(e) => Err(e),
        })
    });
}
