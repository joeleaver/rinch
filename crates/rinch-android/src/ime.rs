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

use jni::objects::JValue;

use crate::bridge;
use crate::jni_exception::{jni_ok, jni_try, native_scope};

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
    /// `deleteSurroundingText(before, after)`, in UTF-16 code units (Android's
    /// unit for this call — `deleteSurroundingTextInCodePoints` is the other
    /// one). Carried raw; converting needs the field's text, which this
    /// connection does not have.
    DeleteSurroundingText { before: i32, after: i32 },
}

static UPDATES: Mutex<Vec<ImeUpdate>> = Mutex::new(Vec::new());

pub fn show_keyboard() {
    bridge::with_activity(|env, activity| {
        jni_ok(env, "showKeyboard", |env| {
            env.call_method(activity, "showKeyboard", "()V", &[])
        });
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
///
/// Returns whether the push actually reached Java. The caller mirrors what it
/// last pushed (to avoid restarting the input session for a field of the same
/// kind), and a mirror advanced past a call that never landed would claim the
/// keyboard had been told something it had not — with nothing left to push it
/// again. So a failed call must not advance it.
#[must_use = "a push that did not land must not advance the caller's mirror"]
pub fn set_multiline(multiline: bool) -> bool {
    bridge::with_activity(|env, activity| {
        jni_ok(env, "setInputMultiline", |env| {
            env.call_method(
                activity,
                "setInputMultiline",
                "(Z)V",
                &[JValue::Bool(multiline as jni::sys::jboolean)],
            )
        })
        .is_some()
    })
}

pub fn hide_keyboard() {
    bridge::with_activity(|env, activity| {
        jni_ok(env, "hideKeyboard", |env| {
            env.call_method(activity, "hideKeyboard", "()V", &[])
        });
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
        jni_ok(env, "restartInput", |env| {
            env.call_method(activity, "restartInput", "()V", &[])
        });
    });
}

/// Take everything the IME has done since the last drain, in order.
pub fn drain_updates() -> Vec<ImeUpdate> {
    std::mem::take(&mut *UPDATES.lock().unwrap())
}

fn push(update: ImeUpdate) {
    UPDATES.lock().unwrap().push(update);
    // Every one of the entry points below runs on the IME's thread, not the
    // frame loop's, and this queue is invisible to the `ALooper` that loop
    // sleeps on. Since K37 that loop sleeps indefinitely on a still screen —
    // and a screen with a soft keyboard on it and a finger typing into it is,
    // to the looper, still. Without this the committed word waits for the next
    // touch.
    crate::wake::wake_main();
}

// ── JNI entry points (called from RinchInputConnection.java) ───────────

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeCommitText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
) {
    // A read that fails still has to leave a mark: this queue's ordering is
    // load-bearing, and dropping the commit that ended a composition would
    // leave the preedit drawn over text that has already replaced it. The empty
    // commit is the IME's own "throw the region away", which is the closest
    // truthful thing to say when the string cannot be read.
    //
    // The scope settles the exception a failed read leaves pending (#419):
    // returned to Java still pending, it would be thrown into the IME's call.
    let _scope = native_scope(&env, "nativeCommitText");
    match jni_try(
        &mut env,
        "commitText: could not read the committed string",
        |env| env.get_string(&text),
    ) {
        Ok(s) => push(ImeUpdate::CommitText(s.into())),
        Err(e) => {
            log::warn!("{e}; clearing instead");
            push(ImeUpdate::CommitText(String::new()));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchInputConnection_nativeSetComposingText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
    new_cursor_position: jni::sys::jint,
) {
    let _scope = native_scope(&env, "nativeSetComposingText");
    match jni_try(
        &mut env,
        "setComposingText: could not read the composing string",
        |env| env.get_string(&text),
    ) {
        Ok(s) => push(ImeUpdate::SetComposingText {
            text: s.into(),
            new_cursor_position,
        }),
        Err(e) => {
            log::warn!("{e}; clearing");
            push(ImeUpdate::SetComposingText {
                text: String::new(),
                new_cursor_position,
            });
        }
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
