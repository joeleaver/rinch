//! Android activity lifecycle events.
//!
//! Register callbacks for `onPause`, `onResume`, `onStop`, and `onStart`.
//! Callbacks fire on the main thread during the next drain cycle.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LifecycleState {
    Created = 0,
    Started = 1,
    Resumed = 2,
    Paused = 3,
    Stopped = 4,
}

static PENDING_EVENT: AtomicU8 = AtomicU8::new(0);
static CURRENT_STATE: Mutex<LifecycleState> = Mutex::new(LifecycleState::Resumed);

thread_local! {
    static ON_PAUSE: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
    static ON_RESUME: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
}

const EVENT_NONE: u8 = 0;
const EVENT_PAUSED: u8 = 1;
const EVENT_RESUMED: u8 = 2;

/// Register a callback that fires when the activity is paused (e.g., user switches apps).
pub fn on_pause(cb: impl Fn() + 'static) {
    ON_PAUSE.with(|slot| *slot.borrow_mut() = Some(Box::new(cb)));
}

/// Register a callback that fires when the activity is resumed (e.g., user returns).
pub fn on_resume(cb: impl Fn() + 'static) {
    ON_RESUME.with(|slot| *slot.borrow_mut() = Some(Box::new(cb)));
}

/// Get the current lifecycle state.
pub fn state() -> LifecycleState {
    *CURRENT_STATE.lock().unwrap()
}

/// Drain lifecycle events and invoke callbacks.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_lifecycle() {
    let event = PENDING_EVENT.swap(EVENT_NONE, Ordering::Relaxed);
    match event {
        EVENT_PAUSED => {
            *CURRENT_STATE.lock().unwrap() = LifecycleState::Paused;
            ON_PAUSE.with(|cb| {
                if let Some(cb) = cb.borrow().as_ref() {
                    cb();
                }
            });
        }
        EVENT_RESUMED => {
            *CURRENT_STATE.lock().unwrap() = LifecycleState::Resumed;
            ON_RESUME.with(|cb| {
                if let Some(cb) = cb.borrow().as_ref() {
                    cb();
                }
            });
        }
        _ => {}
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnPause(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    PENDING_EVENT.store(EVENT_PAUSED, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnResume(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    PENDING_EVENT.store(EVENT_RESUMED, Ordering::Relaxed);
}
