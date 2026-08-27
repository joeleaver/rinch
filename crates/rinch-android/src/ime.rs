//! Android IME (soft keyboard) bridge.
//!
//! Text input flows: user types on soft keyboard → Android IME →
//! `RinchInputConnection.java` → JNI callback → [`drain_committed_text`].

use std::sync::Mutex;

use jni::objects::JValue;

use crate::bridge;

static COMMITTED_TEXT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static DELETIONS: Mutex<Vec<Deletion>> = Mutex::new(Vec::new());

pub struct Deletion {
    pub before: i32,
    pub after: i32,
}

pub fn show_keyboard() {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "showKeyboard", "()V", &[]) {
            log::warn!("showKeyboard failed: {e}");
        }
    });
}

/// Tell the keyboard whether the focused field takes a line break.
///
/// Android builds the keyboard's `EditorInfo` — which is where the Enter key's
/// meaning is declared — once per input session, so this is not a property the
/// keyboard re-reads. The Java side restarts the session when the value
/// actually changes; a field of the same kind as the last one costs nothing.
///
/// Call this *before* [`show_keyboard`] when focus arrives, so the session the
/// keyboard opens is already the right kind. Both hop to the UI thread through
/// the same handler queue, so they stay in that order.
pub fn set_multiline(multiline: bool) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "setInputMultiline",
            "(Z)V",
            &[JValue::Bool(multiline as jni::sys::jboolean)],
        ) {
            log::warn!("setInputMultiline failed: {e}");
        }
    });
}

pub fn hide_keyboard() {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "hideKeyboard", "()V", &[]) {
            log::warn!("hideKeyboard failed: {e}");
        }
    });
}

pub fn drain_committed_text() -> Vec<String> {
    std::mem::take(&mut *COMMITTED_TEXT.lock().unwrap())
}

pub fn drain_deletions() -> Vec<Deletion> {
    std::mem::take(&mut *DELETIONS.lock().unwrap())
}

// ── JNI entry points (called from RinchInputConnection.java) ───────────

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeCommitText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
) {
    if let Ok(s) = env.get_string(&text) {
        let text: String = s.into();
        COMMITTED_TEXT.lock().unwrap().push(text);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeDeleteSurrounding(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    before: jni::sys::jint,
    after: jni::sys::jint,
) {
    DELETIONS.lock().unwrap().push(Deletion { before, after });
}
