//! Android sensor access (accelerometer, gyroscope, magnetometer, light, etc.).
//!
//! Call `start()` with a sensor type and callback to begin receiving data.
//! The callback fires on the main thread each frame with the latest reading.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;

use crate::scoped::ScopedMap;

#[cfg(target_os = "android")]
use jni::objects::JValue;

#[cfg(target_os = "android")]
use crate::bridge;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SensorType {
    Accelerometer = 1,
    MagneticField = 2,
    Gyroscope = 4,
    Light = 5,
    Pressure = 6,
    Proximity = 8,
    StepCounter = 19,
}

#[derive(Clone, Debug)]
pub struct SensorData {
    pub values: [f32; 6],
    pub num_values: usize,
    pub timestamp_ns: u64,
}

/// ~5 Hz
pub const DELAY_NORMAL: i32 = 200_000;
/// ~16 Hz, good for UI updates
pub const DELAY_UI: i32 = 60_000;
/// ~50 Hz, good for games
pub const DELAY_GAME: i32 = 20_000;
/// Maximum sensor rate
pub const DELAY_FASTEST: i32 = 0;

static SENSOR_DATA: Mutex<Option<HashMap<i32, SensorData>>> = Mutex::new(None);

thread_local! {
    /// Scope-aware (issue #183): a callback registered inside a component stops
    /// firing, and is released, once that component unmounts — `stop()` used to
    /// be the only thing that removed one.
    static SENSOR_CALLBACKS: ScopedMap<i32, dyn Fn(&SensorData)> = ScopedMap::new();
}

/// Start receiving sensor data. The callback fires on the main thread each frame
/// with the latest reading. `delay_us` controls the update rate (use `DELAY_*` constants).
pub fn start(sensor_type: SensorType, delay_us: i32, cb: impl Fn(&SensorData) + 'static) {
    let type_id = sensor_type as i32;
    SENSOR_CALLBACKS.with(|map| map.install(type_id, Rc::new(cb)));
    arm_sensor(type_id, delay_us);
}

/// Ask the platform to start delivering this sensor. The JNI half of [`start`].
#[cfg(target_os = "android")]
fn arm_sensor(type_id: i32, delay_us: i32) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(
            activity,
            "startSensor",
            "(II)V",
            &[JValue::Int(type_id), JValue::Int(delay_us)],
        ) {
            log::warn!("startSensor({type_id}) JNI call failed: {e}");
        }
    });
}

#[cfg(not(target_os = "android"))]
fn arm_sensor(_type_id: i32, _delay_us: i32) {}

/// Stop receiving sensor data for the given type.
pub fn stop(sensor_type: SensorType) {
    let type_id = sensor_type as i32;
    SENSOR_CALLBACKS.with(|map| map.remove(&type_id));
    disarm_sensor(type_id);
}

/// Ask the platform to stop delivering this sensor. The JNI half of [`stop`].
#[cfg(target_os = "android")]
fn disarm_sensor(type_id: i32) {
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "stopSensor", "(I)V", &[JValue::Int(type_id)]) {
            log::warn!("stopSensor({type_id}) JNI call failed: {e}");
        }
    });
}

#[cfg(not(target_os = "android"))]
fn disarm_sensor(_type_id: i32) {}

/// Whether a callback is currently registered for `sensor_type`.
#[cfg(test)]
fn callback_registered(sensor_type: SensorType) -> bool {
    SENSOR_CALLBACKS.with(|map| map.contains(&(sensor_type as i32)))
}

/// Drain latest sensor values and invoke registered callbacks.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_sensor_events() {
    // Release what unmounted components left behind, whether or not a reading
    // arrived. A sensor that has fallen silent is never dispatched again, so
    // pruning only on dispatch would hold a dead callback — and everything it
    // captured — for the life of the process. A `Weak` upgrade per registered
    // sensor, of which there are at most a handful.
    //
    // Logged, because a release is otherwise completely silent: the callback
    // simply stops firing, which is exactly the symptom someone would come here
    // to explain.
    let released = SENSOR_CALLBACKS.with(|map| map.release_dead());
    if released > 0 {
        log::debug!("Released {released} sensor callback(s) whose component is gone");
    }

    let snapshot: HashMap<i32, SensorData> = {
        let mut guard = SENSOR_DATA.lock().unwrap();
        guard.take().unwrap_or_default()
    };
    if snapshot.is_empty() {
        return;
    }
    for (type_id, data) in &snapshot {
        // One short borrow per callback, released before it is called: stopping
        // a sensor from inside its own reading is ordinary use, and it re-enters
        // this registry.
        SENSOR_CALLBACKS.with(|map| map.dispatch(type_id, |cb| cb(data)));
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeOnSensorChanged(
    #[allow(unused_mut)] mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    sensor_type: jni::sys::jint,
    values: jni::objects::JFloatArray,
    timestamp: jni::sys::jlong,
) {
    let len = env.get_array_length(&values).unwrap_or(0) as usize;
    let num = len.min(6);
    let mut vals = [0.0f32; 6];
    if num > 0 {
        env.get_float_array_region(&values, 0, &mut vals[..num])
            .ok();
    }
    record_reading(
        sensor_type,
        SensorData {
            values: vals,
            num_values: num,
            timestamp_ns: timestamp as u64,
        },
    );
}

/// Record a reading for delivery by the next [`drain_sensor_events`].
///
/// The host-compiled half of the JNI entry point above, so the drain path can be
/// exercised by tests on a machine with no device attached.
#[cfg(any(target_os = "android", test))]
fn record_reading(sensor_type: i32, data: SensorData) {
    SENSOR_DATA
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(sensor_type, data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::Signal;
    use rinch_core::reactive::Scope;
    use std::cell::Cell;
    use std::rc::Rc;

    fn reading(v: f32) -> SensorData {
        SensorData {
            values: [v, 0.0, 0.0, 0.0, 0.0, 0.0],
            num_values: 1,
            timestamp_ns: 0,
        }
    }

    /// A callback registered while a component was rendering must not run once
    /// that component is gone: it captured the component's `Signal`s, disposal
    /// freed them, and a *read* of a freed signal panics (issue #183, #141 PR4).
    #[test]
    fn a_sensor_callback_registered_in_a_scope_is_not_invoked_after_the_scope_disposes() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(|| {
            start(SensorType::Light, DELAY_UI, move |_| flag.set(true));
        });

        scope.dispose();
        record_reading(SensorType::Light as i32, reading(1.0));
        drain_sensor_events();

        assert!(
            !ran.get(),
            "a sensor callback registered by a since-disposed scope must not run"
        );
        assert!(
            !callback_registered(SensorType::Light),
            "the dead entry must be pruned, or every later reading re-checks it"
        );
    }

    /// A sensor that has fallen silent never dispatches again, so a dead entry
    /// would hold everything its callback captured for the life of the process
    /// even though the drain runs every frame.
    #[test]
    fn a_dead_sensor_callback_is_released_even_if_that_sensor_never_reports_again() {
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
            start(SensorType::Pressure, DELAY_UI, move |_| {
                let _keep = &spy;
            });
        });

        scope.dispose();
        // No reading for Pressure — only the drain running, as it does each frame.
        drain_sensor_events();

        assert!(
            dropped.get(),
            "the dead callback must be released without waiting for a reading \
             that may never come"
        );
    }

    /// Registration from `main`, from startup code or from a detached callback
    /// has no ambient owner and therefore app lifetime — the pre-#141 default,
    /// which the liveness check must not disturb.
    #[test]
    fn a_sensor_callback_registered_with_no_ambient_owner_still_runs() {
        let _serial = crate::test_serial();

        let seen = Rc::new(Cell::new(0.0f32));
        let s = seen.clone();
        // Deliberately not inside a `Scope::run`.
        start(SensorType::Proximity, DELAY_UI, move |d| s.set(d.values[0]));

        record_reading(SensorType::Proximity as i32, reading(7.5));
        drain_sensor_events();

        assert_eq!(seen.get(), 7.5, "an ownerless callback keeps app lifetime");
        stop(SensorType::Proximity);
    }

    /// The callback runs with its registering component as the ambient owner, so
    /// whatever it allocates belongs to that component rather than to whatever
    /// the event loop happened to be doing.
    #[test]
    fn a_live_sensor_callback_runs_with_its_component_as_ambient_owner() {
        let _serial = crate::test_serial();

        let scope = Scope::new();
        scope.run(|| {
            start(SensorType::Gyroscope, DELAY_UI, |_| {
                let _owned_by_the_component = Signal::new(0u32);
            });
        });

        let before = scope.owned_counts().signals;
        record_reading(SensorType::Gyroscope as i32, reading(1.0));
        drain_sensor_events();
        let after = scope.owned_counts().signals;

        assert_eq!(
            after,
            before + 1,
            "a signal created inside the callback must be attributed to the \
             scope that registered it"
        );
        scope.dispose();
        // Leave no dead entry behind for a runner that shares a thread.
        stop(SensorType::Gyroscope);
    }

    /// "Stop when the reading crosses a threshold" is the obvious use of a sensor
    /// callback, and it re-enters the registry. Holding the map's borrow across
    /// the call makes it a `BorrowMutError`.
    #[test]
    fn a_sensor_callback_may_stop_its_own_sensor_from_inside_its_dispatch() {
        let _serial = crate::test_serial();

        let ran = Rc::new(Cell::new(0u32));
        let n = ran.clone();
        start(SensorType::StepCounter, DELAY_UI, move |_| {
            n.set(n.get() + 1);
            stop(SensorType::StepCounter);
        });

        record_reading(SensorType::StepCounter as i32, reading(1.0));
        drain_sensor_events();
        record_reading(SensorType::StepCounter as i32, reading(2.0));
        drain_sensor_events();

        assert_eq!(
            ran.get(),
            1,
            "stop() from inside the callback must take effect"
        );
    }

    /// An unmounted component must not silence a live one: the entries are
    /// independent, and pruning one must leave the other armed.
    #[test]
    fn a_dead_sensor_entry_does_not_take_a_live_sibling_with_it() {
        let _serial = crate::test_serial();

        let live_ran = Rc::new(Cell::new(false));
        let live_flag = live_ran.clone();
        let live = Scope::new();
        live.run(|| {
            start(SensorType::MagneticField, DELAY_UI, move |_| {
                live_flag.set(true)
            });
        });

        let dead = Scope::new();
        dead.run(|| {
            start(SensorType::Accelerometer, DELAY_UI, |_| {});
        });
        dead.dispose();

        record_reading(SensorType::MagneticField as i32, reading(1.0));
        drain_sensor_events();

        assert!(
            live_ran.get(),
            "the live component's sensor must still fire"
        );
        live.dispose();
    }
}
