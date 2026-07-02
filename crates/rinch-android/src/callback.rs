//! Android activity result and permission callback routing.
//!
//! JNI callbacks push raw results into `Mutex<Vec<T>>` queues from any thread.
//! `drain_*()` functions run on the main thread each frame, matching results
//! to registered `FnOnce` callbacks via request code.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};

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
    static ACTIVITY_CALLBACKS: RefCell<HashMap<i32, Box<dyn FnOnce(ActivityResult)>>> =
        RefCell::new(HashMap::new());
    static PERMISSION_CALLBACKS: RefCell<HashMap<i32, Box<dyn FnOnce(PermissionResult)>>> =
        RefCell::new(HashMap::new());
}

/// Register a callback for an activity result with the given request code.
/// The callback will fire on the main thread when `drain_activity_results()` runs.
pub fn register_activity_callback(code: i32, cb: impl FnOnce(ActivityResult) + 'static) {
    ACTIVITY_CALLBACKS.with(|map| {
        map.borrow_mut().insert(code, Box::new(cb));
    });
}

/// Register a callback for a permission result with the given request code.
/// The callback will fire on the main thread when `drain_permission_results()` runs.
pub fn register_permission_callback(code: i32, cb: impl FnOnce(PermissionResult) + 'static) {
    PERMISSION_CALLBACKS.with(|map| {
        map.borrow_mut().insert(code, Box::new(cb));
    });
}

// ── Drain functions (called from android_runtime.rs main loop) ──────────────

/// Drain pending activity results and invoke matched callbacks.
pub fn drain_activity_results() {
    let results: Vec<ActivityResult> = std::mem::take(&mut *ACTIVITY_RESULTS.lock().unwrap());
    for result in results {
        let cb = ACTIVITY_CALLBACKS.with(|map| map.borrow_mut().remove(&result.request_code));
        if let Some(cb) = cb {
            cb(result);
        } else {
            log::warn!(
                "No callback registered for activity result (request_code={})",
                result.request_code
            );
        }
    }
}

/// Drain pending permission results and invoke matched callbacks.
pub fn drain_permission_results() {
    let results: Vec<PermissionResult> = std::mem::take(&mut *PERMISSION_RESULTS.lock().unwrap());
    for result in results {
        let cb = PERMISSION_CALLBACKS.with(|map| map.borrow_mut().remove(&result.request_code));
        if let Some(cb) = cb {
            cb(result);
        } else {
            log::warn!(
                "No callback registered for permission result (request_code={})",
                result.request_code
            );
        }
    }
}

// ── JNI entry points ────────────────────────────────────────────────────────

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

    ACTIVITY_RESULTS.lock().unwrap().push(ActivityResult {
        request_code,
        result_code,
        data_uri: uri,
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnPermissionsResult(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
    request_code: jni::sys::jint,
    all_granted: jni::sys::jboolean,
) {
    PERMISSION_RESULTS.lock().unwrap().push(PermissionResult {
        request_code,
        all_granted: all_granted != 0,
    });
}
