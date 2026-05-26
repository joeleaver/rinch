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
