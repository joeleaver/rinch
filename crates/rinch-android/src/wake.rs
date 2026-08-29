//! Waking the shell's frame loop from a thread that is not it.
//!
//! **Why this exists.** Until card K37 the Android loop polled with a fixed
//! 16ms timeout, which meant it woke roughly sixty-two times a second whether
//! or not anything had happened. That timeout was doing two jobs at once: it
//! was the frame clock *and* it was the polling interval for every queue in
//! this crate that a Java thread writes and the loop reads — the IME's
//! `InputConnection` calls, `onActivityResult`, `onRequestPermissionsResult`,
//! `onSensorChanged`, `onLocationChanged`. None of those go through the
//! `ALooper` the loop is blocked on; they land in a `Mutex` and wait to be
//! noticed, and what noticed them was the timeout expiring.
//!
//! K37 took the timeout away: the loop now sleeps until something happens,
//! because a phone that wakes sixty-two times a second on a still screen is
//! spending battery to discover that nothing has changed. That is the right
//! thing to do to the frame clock and it silently breaks every queue above —
//! a committed IME word, a returned file picker, a location fix would all sit
//! in their mutex until the user happened to touch the screen.
//!
//! So each of those producers now says so. `android-activity` hands out an
//! [`AndroidAppWaker`] whose `wake()` is `ALooper_wake`, an increment of the
//! looper's wake eventfd, safe to call from any thread. A `wake()` that lands
//! *before* the loop calls `ALooper_pollAll` is not lost: an eventfd holds a
//! count until somebody reads it, so the poll returns immediately rather than
//! blocking on a signal that has already been and gone. There is no window
//! between "check the queue" and "go to sleep" for a producer to fall into,
//! which matters because that window is exactly where a missed wake becomes a
//! frozen screen and not a late one.
//!
//! One producer in the process cannot be fixed this way and is not listed
//! above: `rinch_core::reactive::poll_signal` bridges a background thread's
//! atomic into a signal by having the frame loop *read* it once a frame, so
//! there is nothing on the writing side to ring a waker — nobody there knows a
//! read is owed. That one is handled by the loop keeping its own alarm clock;
//! see `next_poll_due` and `shell::android_frame::poll_timeout`.
//!
//! Nothing here is required for correctness on the *main* thread: code already
//! running inside the loop will be seen by the rest of that same iteration.
//! [`wake_main`] is for the JNI entry points, which run on whichever thread
//! Android felt like calling them from.

use std::sync::OnceLock;

use android_activity::{AndroidApp, AndroidAppWaker};

static WAKER: OnceLock<AndroidAppWaker> = OnceLock::new();

/// Remember how to wake the frame loop. Called once by [`crate::init`], from
/// `android_main`, before any JNI callback can have fired.
pub(crate) fn install(android_app: &AndroidApp) {
    let _ = WAKER.set(android_app.create_waker());
}

/// Wake the shell's frame loop out of `poll_events`, from any thread.
///
/// A no-op before [`crate::init`] and in a host process that has no loop —
/// both cases mean there is nothing blocked to wake, not that a wake was
/// missed.
pub fn wake_main() {
    if let Some(waker) = WAKER.get() {
        waker.wake();
    }
}
