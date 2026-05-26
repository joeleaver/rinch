//! Stub clipboard implementation for Android.
//! TODO: Implement via JNI bridge to ClipboardManager.

use super::{ClipboardError, ClipboardResult, ImageData};

pub fn copy_text(_text: impl AsRef<str>) -> ClipboardResult<()> {
    Err(ClipboardError::NotSupported)
}

pub fn paste_text() -> ClipboardResult<String> {
    Err(ClipboardError::NotSupported)
}

pub fn has_text() -> bool {
    false
}

pub fn clear() -> ClipboardResult<()> {
    Err(ClipboardError::NotSupported)
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
