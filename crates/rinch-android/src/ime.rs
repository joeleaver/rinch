//! Android IME (soft keyboard) bridge.
//!
//! Text input flows: user types on soft keyboard → Android IME →
//! `RinchInputConnection.java` → JNI callback → [`drain_updates`].
//!
//! This module is transport and nothing else: it queues the calls the IME made,
//! in the order it made them, and hands them over untouched. What they *mean* —
//! the composing region, when a composition ends, what a commit does to the one
//! that preceded it — is `rinch::shell::android_ime`, which is pure and tested
//! on the host. Nothing here interprets.

use std::sync::Mutex;

use crate::bridge;

/// One call the IME made on `RinchInputConnection`, queued in call order.
///
/// One queue rather than several. Composing and committing are a *sequence* —
/// `setComposingText` opens a region and `commitText`/`finishComposingText`
/// ends it — so a commit drained ahead of the composition it ended would clear
/// a preedit that had not been drawn yet, and one drained behind it would leave
/// the composition on screen beside the text that replaced it. Separate queues
/// (which is what `drain_committed_text` and `drain_deletions` were) cannot
/// order the two against each other at all.
#[derive(Debug, Clone, PartialEq)]
pub enum ImeUpdate {
    /// `setComposingText(text, newCursorPosition)` — the composing region is
    /// now `text`, whole. `new_cursor_position` is carried raw: it is
    /// character-relative and can name a position outside the region, which is
    /// the consumer's problem to resolve.
    SetComposingText {
        text: String,
        new_cursor_position: i32,
    },
    /// `finishComposingText()` — stop composing; the region's text stands.
    FinishComposingText,
    /// `commitText(text, _)` — real text at the caret, replacing any composing
    /// region. Empty is meaningful: it discards the region.
    CommitText(String),
    /// `deleteSurroundingText(before, after)`, in characters.
    DeleteSurroundingText { before: i32, after: i32 },
}

static UPDATES: Mutex<Vec<ImeUpdate>> = Mutex::new(Vec::new());

pub fn show_keyboard() {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "showKeyboard", "()V", &[]) {
            log::warn!("showKeyboard failed: {e}");
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

/// Make the IME start over on the focused field.
///
/// Called when a composition is abandoned because focus moved between two of
/// rinch's fields. Android cannot see that move — there is one
/// `RinchInputView` and it keeps focus throughout — so without this the
/// keyboard goes on composing a word that belongs to a field which no longer
/// has focus, and delivers it into the next one.
pub fn restart_input() {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "restartInput", "()V", &[]) {
            log::warn!("restartInput failed: {e}");
        }
    });
}

/// Take everything the IME has done since the last drain, in order.
pub fn drain_updates() -> Vec<ImeUpdate> {
    std::mem::take(&mut *UPDATES.lock().unwrap())
}

fn push(update: ImeUpdate) {
    UPDATES.lock().unwrap().push(update);
}

// ── JNI entry points (called from RinchInputConnection.java) ───────────

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeCommitText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
) {
    if let Ok(s) = env.get_string(&text) {
        push(ImeUpdate::CommitText(s.into()));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeSetComposingText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
    new_cursor_position: jni::sys::jint,
) {
    if let Ok(s) = env.get_string(&text) {
        push(ImeUpdate::SetComposingText {
            text: s.into(),
            new_cursor_position,
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeFinishComposingText(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    push(ImeUpdate::FinishComposingText);
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeDeleteSurrounding(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    before: jni::sys::jint,
    after: jni::sys::jint,
) {
    push(ImeUpdate::DeleteSurroundingText { before, after });
}
