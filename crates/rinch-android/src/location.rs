//! Android GPS/network location via LocationManager.
//!
//! Call `start()` to begin receiving location updates. The callback fires on the
//! main thread when a new fix arrives. Automatically requests `ACCESS_FINE_LOCATION`
//! permission if needed.

use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::scoped::ScopedSlot;

#[cfg(target_os = "android")]
use jni::objects::JValue;

#[cfg(target_os = "android")]
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
    /// Scope-aware (issue #183): a callback registered inside a component stops
    /// firing, and is released, once that component unmounts — `stop()` used to
    /// be the only thing that removed it.
    static LOCATION_CALLBACK: ScopedSlot<dyn Fn(&LocationData)> = const { ScopedSlot::new() };
}

/// Start location updates. Requests `ACCESS_FINE_LOCATION` permission if needed.
///
/// - `min_time_ms`: minimum time between updates in milliseconds (0 = fastest)
/// - `min_distance_m`: minimum distance between updates in meters (0 = any change)
/// - `cb`: fires on the main thread with each new location fix
pub fn start(min_time_ms: u64, min_distance_m: f32, cb: impl Fn(&LocationData) + 'static) {
    LOCATION_CALLBACK.with(|slot| slot.install(Rc::new(cb)));

    arm_updates(min_time_ms, min_distance_m);
}

/// Ask the platform for location updates, requesting the permission first if it
/// is not already held. The JNI half of [`start`].
#[cfg(target_os = "android")]
fn arm_updates(min_time_ms: u64, min_distance_m: f32) {
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

#[cfg(not(target_os = "android"))]
fn arm_updates(_min_time_ms: u64, _min_distance_m: f32) {}

#[cfg(target_os = "android")]
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
    LOCATION_CALLBACK.with(|slot| slot.clear());
    disarm_updates();
}

/// Ask the platform to stop location updates. The JNI half of [`stop`].
#[cfg(target_os = "android")]
fn disarm_updates() {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "stopLocationUpdates", "()V", &[]) {
            log::warn!("stopLocationUpdates JNI call failed: {e}");
        }
    });
}

#[cfg(not(target_os = "android"))]
fn disarm_updates() {}

/// Whether a location callback is currently installed.
#[cfg(test)]
fn callback_installed() -> bool {
    LOCATION_CALLBACK.with(|slot| slot.is_installed())
}

/// Get the last known location (if any).
pub fn last_known() -> Option<LocationData> {
    LOCATION.lock().unwrap().clone()
}

/// Drain location updates and invoke the registered callback.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_location() {
    // Release what an unmounted component left behind, whether or not a fix
    // arrived: a device that loses its fix never dispatches again, so pruning
    // only on dispatch would hold a dead callback for the life of the process.
    // Logged, because a release is otherwise silent — the callback just stops.
    if LOCATION_CALLBACK.with(|slot| slot.release_if_dead()) {
        log::debug!("Released the location callback: the component that started it is gone");
    }

    if !LOCATION_CHANGED.swap(false, Ordering::Relaxed) {
        return;
    }
    let data = LOCATION.lock().unwrap().clone();
    if let Some(data) = data {
        // The borrow is released before the call: stopping updates from inside
        // the fix that satisfied you is ordinary use, and it re-enters the slot.
        LOCATION_CALLBACK.with(|slot| slot.dispatch(|cb| cb(&data)));
    }
}

#[cfg(target_os = "android")]
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

    record_fix(LocationData {
        latitude: lat,
        longitude: lon,
        altitude: alt,
        accuracy,
        speed,
        bearing,
        timestamp_ms: timestamp as u64,
        provider: provider_str,
    });
}

/// Record a fix for delivery by the next [`drain_location`].
///
/// The host-compiled half of the JNI entry point above, so the drain path can be
/// exercised by tests on a machine with no device attached.
#[cfg(any(target_os = "android", test))]
fn record_fix(data: LocationData) {
    *LOCATION.lock().unwrap() = Some(data);
    LOCATION_CHANGED.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;
    use std::rc::Rc;

    fn fix(lat: f64) -> LocationData {
        LocationData {
            latitude: lat,
            longitude: 0.0,
            altitude: 0.0,
            accuracy: 0.0,
            speed: 0.0,
            bearing: 0.0,
            timestamp_ms: 0,
            provider: "test".into(),
        }
    }

    /// A callback registered while a component was rendering must not run once
    /// that component is gone: it captured the component's `Signal`s, disposal
    /// freed them, and a *read* of a freed signal panics (issue #183, #141 PR4).
    #[test]
    fn a_location_callback_registered_in_a_scope_is_not_invoked_after_the_scope_disposes() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| start(0, 0.0, move |_| flag.set(true)));

        scope.dispose();
        record_fix(fix(1.0));
        drain_location();

        assert!(
            !ran.get(),
            "a location callback registered by a since-disposed scope must not run"
        );
        assert!(
            !callback_installed(),
            "the dead callback must be pruned, or every later fix re-checks it"
        );
    }

    /// A device that loses its fix never dispatches again, so a dead callback
    /// would hold everything it captured for the life of the process even though
    /// the drain runs every frame.
    #[test]
    fn a_dead_location_callback_is_released_even_if_no_further_fix_arrives() {
        struct DropSpy(Rc<Cell<bool>>);
        impl Drop for DropSpy {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let _serial = crate::test_serial();

        let dropped = Rc::new(Cell::new(false));
        let spy = DropSpy(dropped.clone());
        let scope = Scope::new();
        scope.run(|| {
            start(0, 0.0, move |_| {
                let _keep = &spy;
            })
        });

        scope.dispose();
        // No fix — only the drain running, as it does each frame.
        drain_location();

        assert!(
            dropped.get(),
            "the dead callback must be released without waiting for a fix that \
             may never come"
        );
    }

    /// Registration from `main`, from startup code or from a detached callback
    /// has no ambient owner and therefore app lifetime — the pre-#141 default,
    /// which the liveness check must not disturb.
    #[test]
    fn a_location_callback_registered_with_no_ambient_owner_still_runs() {
        let _serial = crate::test_serial();

        let seen = Rc::new(Cell::new(0.0f64));
        let s = seen.clone();
        // Deliberately not inside a `Scope::run`.
        start(0, 0.0, move |d| s.set(d.latitude));

        record_fix(fix(51.5));
        drain_location();

        assert_eq!(seen.get(), 51.5, "an ownerless callback keeps app lifetime");
        stop();
    }

    /// The callback runs with its registering component as the ambient owner, so
    /// whatever it allocates belongs to that component rather than to whatever
    /// the event loop happened to be doing.
    #[test]
    fn a_live_location_callback_runs_with_its_component_as_ambient_owner() {
        let _serial = crate::test_serial();

        let scope = Scope::new();
        scope.run(|| {
            start(0, 0.0, |_| {
                let _owned_by_the_component = Signal::new(0u32);
            })
        });

        let before = scope.owned_counts().signals;
        record_fix(fix(1.0));
        drain_location();
        let after = scope.owned_counts().signals;

        assert_eq!(
            after,
            before + 1,
            "a signal created inside the callback must be attributed to the \
             scope that registered it"
        );
        scope.dispose();
    }

    /// "Stop once we have a good enough fix" is the obvious use of a location
    /// callback, and it re-enters the slot. Holding the borrow across the call
    /// makes it a `BorrowMutError`.
    #[test]
    fn a_location_callback_may_stop_updates_from_inside_its_dispatch() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(0u32));
        let n = ran.clone();
        start(0, 0.0, move |_| {
            n.set(n.get() + 1);
            stop();
        });

        record_fix(fix(1.0));
        drain_location();
        record_fix(fix(2.0));
        drain_location();

        assert_eq!(
            ran.get(),
            1,
            "stop() from inside the callback must take effect"
        );
    }
}
