//! Android clipboard via ClipboardManager JNI calls.

use jni::objects::JValue;

use crate::bridge;

pub fn copy_text(text: &str) -> Result<(), String> {
    bridge::with_activity(|env, activity| {
        let jtext = env.new_string(text).map_err(|e| e.to_string())?;
        env.call_method(
            activity,
            "copyToClipboard",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&jtext)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub fn paste_text() -> Result<String, String> {
    bridge::with_activity(|env, activity| {
        let result = env
            .call_method(activity, "pasteFromClipboard", "()Ljava/lang/String;", &[])
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?;

        if result.is_null() {
            return Ok(String::new());
        }

        let jstr: jni::objects::JString = result.into();
        let text = env.get_string(&jstr).map_err(|e| e.to_string())?;
        Ok(text.into())
    })
}

pub fn has_text() -> bool {
    bridge::with_activity(|env, activity| {
        env.call_method(activity, "hasClipboardText", "()Z", &[])
            .and_then(|v| v.z())
            .unwrap_or(false)
    })
}
