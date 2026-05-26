//! Gallery image picker and camera capture.
//!
//! - `pick_image` opens the SAF image picker (gallery)
//! - `take_photo` launches the system camera app
//! - `bytes_to_data_uri` converts image bytes to a data URI for `<img src>`

use jni::objects::JValue;

use crate::{bridge, callback, file_picker};

const RESULT_OK: i32 = -1;

/// Pick an image from the gallery. Callback receives JPEG/PNG bytes or `None` if cancelled.
pub fn pick_image(cb: impl FnOnce(Option<Vec<u8>>) + 'static) {
    let code = callback::next_request_code();
    callback::register_activity_callback(code, move |result| {
        if result.result_code == RESULT_OK {
            if let Some(uri) = result.data_uri {
                match file_picker::read_content_uri(&uri) {
                    Ok(bytes) => return cb(Some(bytes)),
                    Err(e) => log::warn!("pick_image: read_content_uri failed: {e}"),
                }
            }
        }
        cb(None)
    });
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "openImagePicker", "(I)V", &[JValue::Int(code)]) {
            log::warn!("openImagePicker JNI call failed: {e}");
        }
    });
}

/// Take a photo with the device camera. Requests CAMERA permission if needed.
/// Callback receives JPEG bytes or `None` if cancelled/denied.
pub fn take_photo(cb: impl FnOnce(Option<Vec<u8>>) + 'static) {
    if !crate::permissions::has_permission("android.permission.CAMERA") {
        crate::permissions::request_permission("android.permission.CAMERA", move |granted| {
            if granted {
                take_photo_inner(cb);
            } else {
                cb(None);
            }
        });
    } else {
        take_photo_inner(cb);
    }
}

fn take_photo_inner(cb: impl FnOnce(Option<Vec<u8>>) + 'static) {
    let code = callback::next_request_code();
    callback::register_activity_callback(code, move |result| {
        if result.result_code == RESULT_OK {
            if let Some(uri) = result.data_uri {
                match file_picker::read_content_uri(&uri) {
                    Ok(bytes) => return cb(Some(bytes)),
                    Err(e) => log::warn!("take_photo: read_content_uri failed: {e}"),
                }
            }
        }
        cb(None)
    });
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "takePhoto", "(I)V", &[JValue::Int(code)]) {
            log::warn!("takePhoto JNI call failed: {e}");
        }
    });
}

/// Convert image bytes (JPEG, PNG, etc.) to a `data:` URI for use with `<img src>`.
pub fn bytes_to_data_uri(bytes: &[u8]) -> String {
    use base64::Engine;
    let mime = if bytes.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else {
        "application/octet-stream"
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}
