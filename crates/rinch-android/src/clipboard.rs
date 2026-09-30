//! Android clipboard via ClipboardManager JNI calls.

use jni::objects::JValue;

use crate::bridge;
use crate::jni_exception::{jni_ok, jni_try};

pub fn copy_text(text: &str) -> Result<(), String> {
    bridge::with_activity(|env, activity| {
        let jtext = jni_try(env, "copy_text: new_string", |env| env.new_string(text))?;
        jni_try(env, "copyToClipboard", |env| {
            env.call_method(
                activity,
                "copyToClipboard",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&jtext)],
            )
        })?;
        Ok(())
    })
}

pub fn paste_text() -> Result<String, String> {
    bridge::with_activity(|env, activity| {
        let result = jni_try(env, "pasteFromClipboard", |env| {
            env.call_method(activity, "pasteFromClipboard", "()Ljava/lang/String;", &[])?
                .l()
        })?;

        if result.is_null() {
            return Ok(String::new());
        }

        let jstr: jni::objects::JString = result.into();
        let text = jni_try(env, "pasteFromClipboard: get_string", |env| {
            env.get_string(&jstr)
        })?;
        Ok(text.into())
    })
}

pub fn has_text() -> bool {
    bridge::with_activity(|env, activity| {
        jni_ok(env, "hasClipboardText", |env| {
            env.call_method(activity, "hasClipboardText", "()Z", &[])?
                .z()
        })
        .unwrap_or(false)
    })
}
