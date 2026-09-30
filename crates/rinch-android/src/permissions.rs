//! Android runtime permissions.
//!
//! Provides `request_permission` to show the system permission dialog and
//! `has_permission` to check if a permission is already granted.

use jni::objects::JValue;

use crate::jni_exception::jni_ok;
use crate::{bridge, callback};

/// Request a runtime permission. The callback receives `true` if granted,
/// `false` if denied. The system dialog is shown on the UI thread.
pub fn request_permission(permission: &str, cb: impl FnOnce(bool) + 'static) {
    let code = callback::next_request_code();
    callback::register_permission_callback(code, move |result| cb(result.all_granted));
    let permission = permission.to_string();
    bridge::with_activity(|env, activity| {
        let Some(jperm) = jni_ok(env, "Failed to create JNI string for permission", |env| {
            env.new_string(&permission)
        }) else {
            return;
        };
        let Some(string_class) = jni_ok(env, "Failed to find java/lang/String class", |env| {
            env.find_class("java/lang/String")
        }) else {
            return;
        };
        let Some(arr) = jni_ok(env, "Failed to create String[] array", |env| {
            env.new_object_array(1, string_class, &jperm)
        }) else {
            return;
        };
        jni_ok(env, "requestPermissions", |env| {
            env.call_method(
                activity,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(&arr), JValue::Int(code)],
            )
        });
    });
}

/// Check if a permission is currently granted (does not prompt the user).
pub fn has_permission(permission: &str) -> bool {
    bridge::with_activity(|env, activity| {
        let Some(jperm) = jni_ok(
            env,
            "Failed to create JNI string for permission check",
            |env| env.new_string(permission),
        ) else {
            return false;
        };
        let result = jni_ok(env, "checkSelfPermission", |env| {
            env.call_method(
                activity,
                "checkSelfPermission",
                "(Ljava/lang/String;)I",
                &[JValue::Object(&jperm)],
            )?
            .i()
        })
        .unwrap_or(-1);
        result == 0 // PackageManager.PERMISSION_GRANTED
    })
}
