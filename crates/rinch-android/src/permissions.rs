//! Android runtime permissions.
//!
//! Provides `request_permission` to show the system permission dialog and
//! `has_permission` to check if a permission is already granted.

use jni::objects::JValue;

use crate::{bridge, callback};

/// Request a runtime permission. The callback receives `true` if granted,
/// `false` if denied. The system dialog is shown on the UI thread.
pub fn request_permission(permission: &str, cb: impl FnOnce(bool) + 'static) {
    let code = callback::next_request_code();
    callback::register_permission_callback(code, move |result| cb(result.all_granted));
    let permission = permission.to_string();
    bridge::with_activity(|env, activity| {
        let jperm = match env.new_string(&permission) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to create JNI string for permission: {e}");
                return;
            }
        };
        let string_class = match env.find_class("java/lang/String") {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to find java/lang/String class: {e}");
                return;
            }
        };
        let arr = match env.new_object_array(1, string_class, &jperm) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("Failed to create String[] array: {e}");
                return;
            }
        };
        if let Err(e) = env.call_method(
            activity,
            "requestPermissions",
            "([Ljava/lang/String;I)V",
            &[JValue::Object(&arr), JValue::Int(code)],
        ) {
            log::warn!("requestPermissions JNI call failed: {e}");
        }
    });
}

/// Check if a permission is currently granted (does not prompt the user).
pub fn has_permission(permission: &str) -> bool {
    bridge::with_activity(|env, activity| {
        let jperm = match env.new_string(permission) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to create JNI string for permission check: {e}");
                return false;
            }
        };
        let result = env
            .call_method(
                activity,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&jperm)],
            )
            .and_then(|v| v.i())
            .unwrap_or(-1);
        result == 0 // PackageManager.PERMISSION_GRANTED
    })
}
