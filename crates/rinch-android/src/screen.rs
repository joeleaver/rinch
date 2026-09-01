//! Holding the display awake while the app is the thing being looked at.
//!
//! The name is deliberately about the *screen* and not about *waking*. A phone
//! has more than one thing in it that can be asleep — the panel, the CPU, a
//! blocked thread — and a module called `wake` in a UI crate would be a magnet
//! for all three. This one keeps the display lit and does nothing else.
//!
//! # Why the window flag and not a wake lock
//!
//! There are two ways to stop the screen going off and they fail differently.
//!
//! `PowerManager.WakeLock` is a handle held by the *process*. It needs
//! `android.permission.WAKE_LOCK` in the manifest, it keeps its grip while the
//! app is in the background, and releasing it is the caller's job — so every
//! path out of the state that wanted it, including the ones nobody thought of,
//! is a path that can leak it. What a leaked screen lock looks like to the
//! person holding the phone is a battery that was full at the start of the
//! evening and is not at the end, with nothing on screen to explain it.
//!
//! `WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON` is a property of the
//! *window*. It needs no permission, and the system honours it only while that
//! window is the one being shown — background the app and the display timeout
//! comes straight back. Note what that does and does not undo: the *effect* is
//! suspended, the *flag* is not. It stays in the window's `LayoutParams`, so
//! returning to the app holds the screen awake again, and nothing clears it
//! before the activity dies. The worst leak it can produce is therefore a
//! screen that does not sleep for the rest of the session — bounded by the
//! activity, and costing nothing while the user is elsewhere. That asymmetry is
//! the whole argument: a keep-awake API is going to be misused eventually, and
//! this is the one whose misuse cannot follow the user out of the app.
//!
//! An app that genuinely needs the CPU to keep running with the screen off
//! wants neither of these — it wants a foreground service, which is a manifest
//! and a lifecycle rather than a function call, and is not something this crate
//! can wrap.
//!
//! # Idempotent on purpose
//!
//! `addFlags` is an or and `clearFlags` an and-not, so calling
//! [`keep_screen_on`] with the value it already has does nothing at all. The
//! intended shape of a caller is therefore a state mirror rather than an
//! edge-triggered pair — recompute "should the screen be on right now" from
//! whatever the app already knows and write the answer, every time that answer
//! could have changed. A caller that has to remember whether it is currently
//! holding the flag is a caller that will one day forget.

use crate::bridge;

/// Keep the display awake while this activity's window is in front, or stop.
///
/// Fire and forget: the Java side posts the flag change to the UI thread (see
/// `RinchActivity.setKeepScreenOn`, and the note there on why a window flag
/// written from the native frame thread is a race rather than a bug you can
/// see), so this has usually not taken effect by the time it returns. There is
/// nothing to wait for and nothing to report — the next frame the system
/// composes is drawn under the new flag.
///
/// A failed JNI call is logged and swallowed, the way every other service
/// wrapper in this crate handles one. The screen going off early is a
/// disappointment; a panic on the frame thread because the activity happened
/// to be tearing down is a crash.
pub fn keep_screen_on(on: bool) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "setKeepScreenOn",
            "(Z)V",
            &[jni::objects::JValue::Bool(on as jni::sys::jboolean)],
        ) {
            log::warn!("setKeepScreenOn({on}) failed: {e}");
        }
    });
}
