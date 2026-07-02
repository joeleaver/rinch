//! Android GPS/network location via LocationManager.
//!
//! Call `start()` to begin receiving location updates. The callback fires on the
//! main thread when a new fix arrives. Automatically requests `ACCESS_FINE_LOCATION`
//! permission if needed.

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use jni::objects::JValue;

use crate::bridge;

#[derive(Clone, Debug)]
pub struct LocationData {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
    pub accuracy: f32,
    pub speed: f32,
    pub bearing: f32,
    pub timestamp_ms: u64,
    pub provider: String,
}

static LOCATION: Mutex<Option<LocationData>> = Mutex::new(None);
static LOCATION_CHANGED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LOCATION_CALLBACK: RefCell<Option<Box<dyn Fn(&LocationData)>>> = RefCell::new(None);
}

/// Start location updates. Requests `ACCESS_FINE_LOCATION` permission if needed.
///
/// - `min_time_ms`: minimum time between updates in milliseconds (0 = fastest)
/// - `min_distance_m`: minimum distance between updates in meters (0 = any change)
/// - `cb`: fires on the main thread with each new location fix
pub fn start(min_time_ms: u64, min_distance_m: f32, cb: impl Fn(&LocationData) + 'static) {
    LOCATION_CALLBACK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(cb));
    });

    let perm = "android.permission.ACCESS_FINE_LOCATION";
    if !crate::permissions::has_permission(perm) {
        crate::permissions::request_permission(perm, move |granted| {
            if granted {
                start_updates(min_time_ms, min_distance_m);
            } else {
                log::warn!("Location permission denied");
            }
        });
    } else {
        start_updates(min_time_ms, min_distance_m);
    }
}

fn start_updates(min_time_ms: u64, min_distance_m: f32) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "startLocationUpdates",
            "(JF)V",
            &[
                JValue::Long(min_time_ms as i64),
                JValue::Float(min_distance_m),
            ],
        ) {
            log::warn!("startLocationUpdates JNI call failed: {e}");
        }
    });
}

/// Stop receiving location updates.
pub fn stop() {
    LOCATION_CALLBACK.with(|slot| {
        *slot.borrow_mut() = None;
    });
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "stopLocationUpdates", "()V", &[]) {
            log::warn!("stopLocationUpdates JNI call failed: {e}");
        }
    });
}

/// Get the last known location (if any).
pub fn last_known() -> Option<LocationData> {
    LOCATION.lock().unwrap().clone()
}

/// Drain location updates and invoke the registered callback.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_location() {
    if !LOCATION_CHANGED.swap(false, Ordering::Relaxed) {
        return;
    }
    let data = LOCATION.lock().unwrap().clone();
    if let Some(data) = data {
        LOCATION_CALLBACK.with(|cb| {
            if let Some(cb) = cb.borrow().as_ref() {
                cb(&data);
            }
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnLocationChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    lat: jni::sys::jdouble,
    lon: jni::sys::jdouble,
    alt: jni::sys::jdouble,
    accuracy: jni::sys::jfloat,
    speed: jni::sys::jfloat,
    bearing: jni::sys::jfloat,
    timestamp: jni::sys::jlong,
    provider: jni::objects::JString,
) {
    let provider_str = if provider.is_null() {
        "unknown".into()
    } else {
        env.get_string(&provider)
            .map(String::from)
            .unwrap_or_else(|_| "unknown".into())
    };

    *LOCATION.lock().unwrap() = Some(LocationData {
        latitude: lat,
        longitude: lon,
        altitude: alt,
        accuracy,
        speed,
        bearing,
        timestamp_ms: timestamp as u64,
        provider: provider_str,
    });
    LOCATION_CHANGED.store(true, Ordering::Relaxed);
}
