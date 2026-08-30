//! Android activity result and permission callback routing.
//!
//! JNI callbacks push raw results into `Mutex<Vec<T>>` queues from any thread.
//! `drain_*()` functions run on the main thread each frame, matching results
//! to registered `FnOnce` callbacks via request code.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::scoped::{Delivery, ScopedOnceMap};

// ── Request code generation ─────────────────────────────────────────────────

static NEXT_REQUEST_CODE: AtomicI32 = AtomicI32::new(1000);

/// Allocate a unique request code for an activity/permission request.
pub fn next_request_code() -> i32 {
    NEXT_REQUEST_CODE.fetch_add(1, Ordering::Relaxed)
}

// ── Result types ────────────────────────────────────────────────────────────

pub struct ActivityResult {
    pub request_code: i32,
    /// Android RESULT_OK = -1, RESULT_CANCELED = 0
    pub result_code: i32,
    pub data_uri: Option<String>,
}

pub struct PermissionResult {
    pub request_code: i32,
    pub all_granted: bool,
}

// ── Result queues (pushed from JNI, any thread) ─────────────────────────────

static ACTIVITY_RESULTS: Mutex<Vec<ActivityResult>> = Mutex::new(Vec::new());
static PERMISSION_RESULTS: Mutex<Vec<PermissionResult>> = Mutex::new(Vec::new());

// ── Callback registries (main thread only) ──────────────────────────────────

thread_local! {
    /// Scope-aware (issue #183). These are one-shots, removed on delivery — but
    /// removal on delivery bounds the *leak*, not the *lifetime*: Android
    /// delivers a result whatever the user does, so a picker opened by a
    /// component that has since unmounted was still delivered exactly once, into
    /// that component's freed state.
    static ACTIVITY_CALLBACKS: ScopedOnceMap<i32, ActivityResult> = ScopedOnceMap::new();
    static PERMISSION_CALLBACKS: ScopedOnceMap<i32, PermissionResult> = ScopedOnceMap::new();
}

/// Register a callback for an activity result with the given request code.
/// The callback will fire on the main thread when `drain_activity_results()` runs.
pub fn register_activity_callback(code: i32, cb: impl FnOnce(ActivityResult) + 'static) {
    ACTIVITY_CALLBACKS.with(|map| map.install(code, Box::new(cb)));
}

/// Register a callback for a permission result with the given request code.
/// The callback will fire on the main thread when `drain_permission_results()` runs.
pub fn register_permission_callback(code: i32, cb: impl FnOnce(PermissionResult) + 'static) {
    PERMISSION_CALLBACKS.with(|map| map.install(code, Box::new(cb)));
}

// ── Drain functions (called from android_runtime.rs main loop) ──────────────

/// Drain pending activity results and invoke matched callbacks.
pub fn drain_activity_results() {
    // Release what unmounted components left behind. A result that never comes
    // back — the activity was killed, the user never returned — would otherwise
    // pin what its callback captured for the life of the process.
    ACTIVITY_CALLBACKS.with(|map| map.release_dead());

    let results: Vec<ActivityResult> = std::mem::take(&mut *ACTIVITY_RESULTS.lock().unwrap());
    for result in results {
        let code = result.request_code;
        match ACTIVITY_CALLBACKS.with(|map| map.deliver(&code, result)) {
            Delivery::Ran => {}
            Delivery::Dropped => log::debug!(
                "Discarding activity result (request_code={code}): the component \
                 that requested it is gone"
            ),
            Delivery::Unregistered => {
                log::warn!("No callback registered for activity result (request_code={code})")
            }
        }
    }
}

/// Drain pending permission results and invoke matched callbacks.
pub fn drain_permission_results() {
    // As in `drain_activity_results`: a dialog the user never answers would
    // otherwise pin its callback forever.
    PERMISSION_CALLBACKS.with(|map| map.release_dead());

    let results: Vec<PermissionResult> = std::mem::take(&mut *PERMISSION_RESULTS.lock().unwrap());
    for result in results {
        let code = result.request_code;
        match PERMISSION_CALLBACKS.with(|map| map.deliver(&code, result)) {
            Delivery::Ran => {}
            Delivery::Dropped => log::debug!(
                "Discarding permission result (request_code={code}): the component \
                 that requested it is gone"
            ),
            Delivery::Unregistered => {
                log::warn!("No callback registered for permission result (request_code={code})")
            }
        }
    }
}

// ── JNI entry points ────────────────────────────────────────────────────────

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnActivityResult(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_uri: jni::objects::JString,
) {
    let uri = if data_uri.is_null() {
        None
    } else {
        match env.get_string(&data_uri) {
            Ok(s) => Some(String::from(s)),
            Err(e) => {
                log::warn!("Failed to read data_uri JString: {e}");
                None
            }
        }
    };

    queue_activity_result(ActivityResult {
        request_code,
        result_code,
        data_uri: uri,
    });
}

/// Queue an activity result for delivery by the next
/// [`drain_activity_results`]. The host-compiled half of the JNI entry point.
#[cfg(any(target_os = "android", test))]
fn queue_activity_result(result: ActivityResult) {
    ACTIVITY_RESULTS.lock().unwrap().push(result);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnPermissionsResult(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    request_code: jni::sys::jint,
    all_granted: jni::sys::jboolean,
) {
    queue_permission_result(PermissionResult {
        request_code,
        all_granted: all_granted != 0,
    });
}

/// Queue a permission result for delivery by the next
/// [`drain_permission_results`]. The host-compiled half of the JNI entry point.
#[cfg(any(target_os = "android", test))]
fn queue_permission_result(result: PermissionResult) {
    PERMISSION_RESULTS.lock().unwrap().push(result);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;
    use std::rc::Rc;

    fn activity_result(code: i32) -> ActivityResult {
        ActivityResult {
            request_code: code,
            result_code: -1,
            data_uri: None,
        }
    }

    fn permission_result(code: i32) -> PermissionResult {
        PermissionResult {
            request_code: code,
            all_granted: true,
        }
    }

    /// Removing on delivery bounds the *leak*; it does not scope the *lifetime*.
    /// A picker opened from a component that has since unmounted still gets its
    /// result — Android delivers `RESULT_CANCELED` even when the user backs out —
    /// and the callback then reads the component's freed signals, which panics
    /// (issue #183, #141 PR4).
    #[test]
    fn a_one_shot_activity_callback_registered_in_a_scope_is_not_delivered_after_the_scope_disposes()
     {
        let _serial = crate::test_serial();

        let code = next_request_code();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| register_activity_callback(code, move |_| flag.set(true)));

        scope.dispose();
        queue_activity_result(activity_result(code));
        drain_activity_results();

        assert!(
            !ran.get(),
            "a one-shot registered by a since-disposed scope must not be delivered"
        );
    }

    /// The same for the permission queue, which has its own registry.
    #[test]
    fn a_one_shot_permission_callback_registered_in_a_scope_is_not_delivered_after_the_scope_disposes()
     {
        let _serial = crate::test_serial();

        let code = next_request_code();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| register_permission_callback(code, move |_| flag.set(true)));

        scope.dispose();
        queue_permission_result(permission_result(code));
        drain_permission_results();

        assert!(
            !ran.get(),
            "a one-shot registered by a since-disposed scope must not be delivered"
        );
    }

    /// A result that never arrives — the activity was killed, the user never came
    /// back — leaves the callback registered forever, holding everything it
    /// captured. The drain runs every frame and must release it.
    #[test]
    fn a_dead_one_shot_is_released_even_if_its_result_never_arrives() {
        struct DropSpy(Rc<Cell<bool>>);
        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let _serial = crate::test_serial();

        let code = next_request_code();
        let dropped = Rc::new(Cell::new(false));
        let spy = DropSpy(dropped.clone());
        let scope = Scope::new();
        scope.run(|| {
            register_activity_callback(code, move |_| {
                let _keep = &spy;
            })
        });

        scope.dispose();
        // No result queued — only the drain running, as it does each frame.
        drain_activity_results();

        assert!(
            dropped.get(),
            "the dead one-shot must be released without waiting for a result \
             that may never come"
        );
        assert!(
            !ACTIVITY_CALLBACKS.with(|map| map.contains(&code)),
            "the registry entry must go with it"
        );
    }

    /// Registration from `main`, from startup code or from a detached callback
    /// has no ambient owner and therefore app lifetime — the pre-#141 default,
    /// which the liveness check must not disturb.
    #[test]
    fn a_one_shot_registered_with_no_ambient_owner_still_runs() {
        let _serial = crate::test_serial();

        let code = next_request_code();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        // Deliberately not inside a `Scope::run`.
        register_activity_callback(code, move |_| flag.set(true));

        queue_activity_result(activity_result(code));
        drain_activity_results();

        assert!(ran.get(), "an ownerless one-shot keeps app lifetime");
    }

    /// The callback runs with its registering component as the ambient owner, so
    /// whatever it allocates — a `Signal` holding the picked image, say —
    /// belongs to that component rather than to whatever the event loop happened
    /// to be doing.
    #[test]
    fn a_live_one_shot_runs_with_its_component_as_ambient_owner() {
        let _serial = crate::test_serial();

        let code = next_request_code();
        let scope = Scope::new();
        scope.run(|| {
            register_activity_callback(code, |_| {
                let _owned_by_the_component = Signal::new(0u32);
            })
        });

        let before = scope.owned_counts().signals;
        queue_activity_result(activity_result(code));
        drain_activity_results();
        let after = scope.owned_counts().signals;

        assert_eq!(
            after,
            before + 1,
            "a signal created inside the callback must be attributed to the \
             scope that registered it"
        );
        scope.dispose();
    }

    /// Chaining is the normal shape — `take_photo` asks for the CAMERA
    /// permission and registers the activity callback from inside the permission
    /// result. The registry must not be borrowed across the delivery.
    #[test]
    fn a_one_shot_may_register_another_from_inside_its_own_delivery() {
        let _serial = crate::test_serial();

        let first = next_request_code();
        let second = next_request_code();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        register_activity_callback(first, move |_| {
            register_activity_callback(second, move |_| flag.set(true));
        });

        queue_activity_result(activity_result(first));
        drain_activity_results();
        queue_activity_result(activity_result(second));
        drain_activity_results();

        assert!(
            ran.get(),
            "the chained one-shot must be registered and delivered"
        );
    }
}
