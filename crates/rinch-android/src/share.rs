//! Android share intent (ACTION_SEND).
//!
//! Share text or images to other apps via the system share sheet.

use jni::objects::JValue;

use crate::bridge;

/// Share plain text via the system share sheet.
pub fn share_text(text: &str) {
    let text = text.to_string();
    bridge::with_activity(|env, activity| {
        let jtext = match env.new_string(&text) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("share_text: failed to create JNI string: {e}");
                return;
            }
        };
        if let Err(e) = env.call_method(
            activity,
            "shareText",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jtext)],
        ) {
            log::warn!("shareText JNI call failed: {e}");
        }
    });
}

/// Share an image (JPEG/PNG bytes) via the system share sheet.
/// Optionally include text alongside the image.
pub fn share_image(image_bytes: &[u8], text: Option<&str>) {
    let bytes = image_bytes.to_vec();
    let text = text.map(String::from);
    bridge::with_activity(|env, activity| {
        let jbytes = match env.byte_array_from_slice(&bytes) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("share_image: failed to create byte array: {e}");
                return;
            }
        };
        let jtext = match &text {
            Some(t) => match env.new_string(t) {
                Ok(s) => jni::objects::JObject::from(s),
                Err(e) => {
                    log::warn!("share_image: failed to create JNI string: {e}");
                    return;
                }
            },
            None => jni::objects::JObject::null(),
        };
        if let Err(e) = env.call_method(
            activity,
            "shareImage",
            "([BLjava/lang/String;)V",
            &[JValue::Object(&jbytes), JValue::Object(&jtext)],
        ) {
            log::warn!("shareImage JNI call failed: {e}");
        }
    });
}
