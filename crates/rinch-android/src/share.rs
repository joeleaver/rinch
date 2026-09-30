//! Android share intent (ACTION_SEND).
//!
//! Share text or images to other apps via the system share sheet.

use jni::objects::JValue;

use crate::bridge;
use crate::jni_exception::jni_ok;

/// Share plain text via the system share sheet.
pub fn share_text(text: &str) {
    let text = text.to_string();
    bridge::with_activity(|env, activity| {
        let Some(jtext) = jni_ok(env, "share_text: failed to create JNI string", |env| {
            env.new_string(&text)
        }) else {
            return;
        };
        jni_ok(env, "shareText", |env| {
            env.call_method(
                activity,
                "shareText",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&jtext)],
            )
        });
    });
}

/// Share an image (JPEG/PNG bytes) via the system share sheet.
/// Optionally include text alongside the image.
pub fn share_image(image_bytes: &[u8], text: Option<&str>) {
    let bytes = image_bytes.to_vec();
    let text = text.map(String::from);
    bridge::with_activity(|env, activity| {
        let Some(jbytes) = jni_ok(env, "share_image: failed to create byte array", |env| {
            env.byte_array_from_slice(&bytes)
        }) else {
            return;
        };
        let jtext = match &text {
            Some(t) => match jni_ok(env, "share_image: failed to create JNI string", |env| {
                env.new_string(t)
            }) {
                Some(s) => jni::objects::JObject::from(s),
                None => return,
            },
            None => jni::objects::JObject::null(),
        };
        jni_ok(env, "shareImage", |env| {
            env.call_method(
                activity,
                "shareImage",
                "([BLjava/lang/String;)V",
                &[JValue::Object(&jbytes), JValue::Object(&jtext)],
            )
        });
    });
}
