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

thread_local! {
    /// A test's stand-in for `navigator.clipboard` (see [`__set_clipboard_source`]).
    static CLIPBOARD_SOURCE: RefCell<Option<Option<web_sys::Clipboard>>> = const { RefCell::new(None) };
}

/// Point every system-clipboard call at `source` instead of this window's
/// `navigator.clipboard`: `Some(Some(c))` uses `c` (another frame's clipboard,
/// say), `Some(None)` behaves as a context with no clipboard, `None` restores
/// the window's own. For tests, which need a write the browser refuses
/// deterministically (an iframe whose permissions policy denies
/// `clipboard-write`); not part of the API.
#[doc(hidden)]
pub fn __set_clipboard_source(source: Option<Option<web_sys::Clipboard>>) {
    CLIPBOARD_SOURCE.with(|s| *s.borrow_mut() = source);
}

/// The browser's `navigator.clipboard`, if this context has one (it is absent on
/// insecure origins and in some embedded webviews).
fn navigator_clipboard() -> Option<web_sys::Clipboard> {
    if let Some(source) = CLIPBOARD_SOURCE.with(|s| s.borrow().clone()) {
        return source;
    }
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
/// On WASM, this stores the text in the in-app buffer (so a later [`paste_text`]
/// sees it) and asks the browser to write the system clipboard **without
/// waiting** for its answer. `Ok` therefore means the buffer was written, not
/// that the browser accepted the write: a refused permission, a write with no
/// user gesture or focus, or an insecure origin all still answer `Ok` here. Use
/// [`copy_text_then`] to learn what the browser said.
pub fn copy_text(text: impl AsRef<str>) -> ClipboardResult<()> {
    let text = text.as_ref().to_string();
    store_text(&text);
    if let Some(clipboard) = navigator_clipboard() {
        // Awaited, so a refused write is handled rather than reported by the
        // browser as an uncaught rejection.
        wasm_bindgen_futures::spawn_local(async move {
            let _ = write_text(&clipboard, &text).await;
        });
    }
    Ok(())
}

/// [`copy_text`] without waiting — identical here, since the browser write is
/// already fire-and-forget. Present so the async API is the same on every target.
pub fn copy_text_async(text: impl AsRef<str>) {
    let _ = copy_text(text);
}

/// Copy text and report **the browser's answer**: `on_done` runs once the
/// promise `navigator.clipboard.writeText` returned settles — `Ok(())` when it
/// resolved, [`ClipboardError::AccessFailed`] carrying the `DOMException`'s name
/// and message when it rejected (`NotAllowedError` for a refused permission or
/// an unfocused document), [`ClipboardError::NotAvailable`] (synchronously, before
/// this returns) when the context has no `navigator.clipboard` (an insecure
/// origin). The in-app buffer is written either way, as [`copy_text`] writes it.
///
/// `on_done` runs on the browser's single thread, i.e. the UI thread.
pub fn copy_text_then(
    text: impl AsRef<str>,
    on_done: impl FnOnce(ClipboardResult<()>) + Send + 'static,
) {
    let text = text.as_ref().to_string();
    store_text(&text);
    let Some(clipboard) = navigator_clipboard() else {
        on_done(Err(ClipboardError::NotAvailable));
        return;
    };
    // `writeText` is called before this returns (inside the caller's user
    // activation); only its answer is awaited.
    let write = start_write(&clipboard, "writeText", &text.as_str().into());
    wasm_bindgen_futures::spawn_local(async move {
        on_done(finish_write(write).await);
    });
}

fn store_text(text: &str) {
    CLIPBOARD_BUFFER.with(|buf| {
        *buf.borrow_mut() = Some(text.to_string());
    });
}

/// Call `clipboard[method](arg)`: the promise it returned, or an error when the
/// method is missing or not a function, or threw. Function-ness is asked with
/// `typeof`, and the result adopted with `Promise.resolve`, rather than with
/// `instanceof`, which fails for a clipboard of another frame (another realm).
fn start_write(
    clipboard: &web_sys::Clipboard,
    method: &str,
    arg: &wasm_bindgen::JsValue,
) -> ClipboardResult<js_sys::Promise> {
    use wasm_bindgen::JsCast;
    let function = js_sys::Reflect::get(clipboard, &method.into())
        .ok()
        .filter(|f| f.is_function())
        .ok_or(ClipboardError::NotSupported)?
        .unchecked_into::<js_sys::Function>();
    let returned = function.call1(clipboard, arg).map_err(rejection)?;
    Ok(js_sys::Promise::resolve(&returned))
}

/// Await a started write: `Ok(())` when its promise resolved.
async fn finish_write(write: ClipboardResult<js_sys::Promise>) -> ClipboardResult<()> {
    wasm_bindgen_futures::JsFuture::from(write?)
        .await
        .map(|_| ())
        .map_err(rejection)
}

/// `writeText(text)`, awaited.
async fn write_text(clipboard: &web_sys::Clipboard, text: &str) -> ClipboardResult<()> {
    finish_write(start_write(clipboard, "writeText", &text.into())).await
}

/// A rejected clipboard promise as an error: `"<name>: <message>"` for a
/// `DOMException` (or any `Error`), the value's string form otherwise.
fn rejection(reason: wasm_bindgen::JsValue) -> ClipboardError {
    let field = |key: &str| {
        js_sys::Reflect::get(&reason, &key.into())
            .ok()
            .and_then(|v| v.as_string())
    };
    let message = match (field("name"), field("message")) {
        (Some(name), Some(message)) => format!("{name}: {message}"),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => reason
            .as_string()
            .unwrap_or_else(|| "the browser refused the clipboard write".to_string()),
    };
    ClipboardError::AccessFailed(message)
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
/// On WASM, stores both in the in-app buffers and asks the browser to write the
/// system clipboard without waiting: a `ClipboardItem` carrying `text/html` (and
/// `text/plain` when `alt_text` is given), or just `alt_text` through
/// `writeText` where that write cannot be made or the browser refuses it. As with [`copy_text`],
/// `Ok` means the buffers were written; [`copy_html_then`] reports the browser's
/// answer.
pub fn copy_html(html: impl AsRef<str>, alt_text: Option<&str>) -> ClipboardResult<()> {
    let html = html.as_ref();
    store_html(html, alt_text);
    if let Some(clipboard) = navigator_clipboard() {
        let write = start_html_write(&clipboard, html, alt_text);
        let alt = alt_text.map(str::to_string);
        wasm_bindgen_futures::spawn_local(async move {
            let _ = finish_html_write(&clipboard, write, alt.as_deref()).await;
        });
    }
    Ok(())
}

/// [`copy_html`] without waiting — identical here (the browser write is already
/// fire-and-forget). Present so the async API is the same on every target.
pub fn copy_html_async(html: impl AsRef<str>, alt_text: Option<&str>) {
    let _ = copy_html(html, alt_text);
}

/// [`copy_html`], reporting the browser's answer as [`copy_text_then`] does.
///
/// When the `ClipboardItem` write cannot be made (no `ClipboardItem` or
/// `clipboard.write`) or the browser refuses it, `alt_text` is written alone
/// through `writeText` and the answer is that write's; with no `alt_text`, the
/// answer is the html write's error ([`ClipboardError::NotSupported`] when it
/// could not be made at all).
pub fn copy_html_then(
    html: impl AsRef<str>,
    alt_text: Option<&str>,
    on_done: impl FnOnce(ClipboardResult<()>) + Send + 'static,
) {
    let html = html.as_ref();
    store_html(html, alt_text);
    let Some(clipboard) = navigator_clipboard() else {
        on_done(Err(ClipboardError::NotAvailable));
        return;
    };
    let write = start_html_write(&clipboard, html, alt_text);
    let alt = alt_text.map(str::to_string);
    wasm_bindgen_futures::spawn_local(async move {
        on_done(finish_html_write(&clipboard, write, alt.as_deref()).await);
    });
}

fn store_html(html: &str, alt_text: Option<&str>) {
    HTML_CLIPBOARD_BUFFER.with(|buf| {
        *buf.borrow_mut() = Some(html.to_string());
    });
    if let Some(text) = alt_text {
        store_text(text);
    }
}

/// Start the `ClipboardItem` write of an html copy (`clipboard.write`), called
/// before the copy returns so it runs inside the caller's user activation.
fn start_html_write(
    clipboard: &web_sys::Clipboard,
    html: &str,
    alt_text: Option<&str>,
) -> ClipboardResult<js_sys::Promise> {
    let item = clipboard_item(html, alt_text).ok_or(ClipboardError::NotSupported)?;
    start_write(clipboard, "write", &js_sys::Array::of1(&item))
}

/// Await the html write; when it could not be made or was refused, write
/// `alt_text` through `writeText` instead and answer with that.
async fn finish_html_write(
    clipboard: &web_sys::Clipboard,
    write: ClipboardResult<js_sys::Promise>,
    alt_text: Option<&str>,
) -> ClipboardResult<()> {
    match finish_write(write).await {
        Ok(()) => Ok(()),
        Err(e) => match alt_text {
            Some(text) => write_text(clipboard, text).await,
            None => Err(e),
        },
    }
}

/// `new ClipboardItem({"text/html": Blob, "text/plain": Blob})`, or `None`
/// where the browser has no `ClipboardItem` (or refuses to build one).
fn clipboard_item(html: &str, alt_text: Option<&str>) -> Option<web_sys::ClipboardItem> {
    let global = js_sys::global();
    let ctor = js_sys::Reflect::get(&global, &"ClipboardItem".into()).ok()?;
    if ctor.is_undefined() || ctor.is_null() {
        return None;
    }
    let record = js_sys::Object::new();
    let blob = |content: &str, mime: &str| -> Option<web_sys::Blob> {
        let parts = js_sys::Array::of1(&content.into());
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime);
        web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).ok()
    };
    let html_blob: wasm_bindgen::JsValue = blob(html, "text/html")?.into();
    js_sys::Reflect::set(&record, &"text/html".into(), &html_blob).ok()?;
    if let Some(text) = alt_text {
        let text_blob: wasm_bindgen::JsValue = blob(text, "text/plain")?.into();
        js_sys::Reflect::set(&record, &"text/plain".into(), &text_blob).ok()?;
    }
    web_sys::ClipboardItem::new_with_record_from_str_to_blob_promise(&record).ok()
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

/// [`paste_rich`], and the buffered `text/plain` beside an html answer. See the
/// native `paste_rich_with_text`.
pub fn paste_rich_with_text() -> ClipboardResult<(RichPaste, Option<String>)> {
    let rich = paste_rich()?;
    let text = match rich {
        RichPaste::Html(_) => paste_text().ok().filter(|t| !t.is_empty()),
        RichPaste::Image(_) | RichPaste::Text(_) => None,
    };
    Ok((rich, text))
}

/// [`paste_rich_with_text`]; the timeout is unused because the read cannot block.
pub fn paste_rich_with_text_timeout(
    _timeout: Duration,
) -> ClipboardResult<(RichPaste, Option<String>)> {
    paste_rich_with_text()
}

/// [`paste_rich_with_text`] delivered by callback. The html and the text beside
/// it both come from the buffers the `paste` event filled; with no html, the
/// text is read as [`paste_rich_async`] reads it.
pub fn paste_rich_with_text_async(
    on_done: impl FnOnce(ClipboardResult<(RichPaste, Option<String>)>) + Send + 'static,
) {
    paste_rich_async(move |result| {
        on_done(result.map(|rich| {
            let text = match rich {
                RichPaste::Html(_) => paste_text().ok().filter(|t| !t.is_empty()),
                RichPaste::Image(_) | RichPaste::Text(_) => None,
            };
            (rich, text)
        }))
    });
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
