//! Native clipboard implementation using arboard.

use arboard::Clipboard;
use std::sync::Mutex;

use crate::{ClipboardError, ClipboardResult, ImageData};

impl From<arboard::Error> for ClipboardError {
    fn from(err: arboard::Error) -> Self {
        ClipboardError::AccessFailed(err.to_string())
    }
}

// Global clipboard instance (arboard recommends reusing the Clipboard instance)
static CLIPBOARD: Mutex<Option<Clipboard>> = Mutex::new(None);

fn with_clipboard<T, F: FnOnce(&mut Clipboard) -> ClipboardResult<T>>(f: F) -> ClipboardResult<T> {
    let mut guard = CLIPBOARD.lock().unwrap();
    if guard.is_none() {
        *guard = Some(Clipboard::new()?);
    }
    f(guard.as_mut().unwrap())
}

/// Copy text to the clipboard.
pub fn copy_text(text: impl AsRef<str>) -> ClipboardResult<()> {
    with_clipboard(|clipboard| {
        clipboard.set_text(text.as_ref())?;
        Ok(())
    })
}

/// Paste text from the clipboard.
///
/// Returns `Err` if the clipboard doesn't contain text.
pub fn paste_text() -> ClipboardResult<String> {
    with_clipboard(|clipboard| {
        let text = clipboard.get_text()?;
        Ok(text)
    })
}

/// Check if the clipboard contains text.
pub fn has_text() -> bool {
    paste_text().is_ok()
}

/// Clear the clipboard contents.
pub fn clear() -> ClipboardResult<()> {
    with_clipboard(|clipboard| {
        clipboard.clear()?;
        Ok(())
    })
}

/// Copy an image to the clipboard.
///
/// The image data should be in RGBA format.
pub fn copy_image(image: ImageData) -> ClipboardResult<()> {
    with_clipboard(|clipboard| {
        let arboard_image = arboard::ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes,
        };
        clipboard.set_image(arboard_image)?;
        Ok(())
    })
}

/// Paste an image from the clipboard.
///
/// Returns the image data in RGBA format.
pub fn paste_image() -> ClipboardResult<ImageData<'static>> {
    with_clipboard(|clipboard| {
        let image = clipboard.get_image()?;
        Ok(ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes,
        })
    })
}

/// Check if the clipboard contains an image.
pub fn has_image() -> bool {
    paste_image().is_ok()
}
