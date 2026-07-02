//! Android sensor access (accelerometer, gyroscope, magnetometer, light, etc.).
//!
//! Call `start()` with a sensor type and callback to begin receiving data.
//! The callback fires on the main thread each frame with the latest reading.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;

use jni::objects::JValue;

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
    static SENSOR_CALLBACKS: RefCell<HashMap<i32, Box<dyn Fn(&SensorData)>>> =
        RefCell::new(HashMap::new());
}

/// Start receiving sensor data. The callback fires on the main thread each frame
/// with the latest reading. `delay_us` controls the update rate (use `DELAY_*` constants).
pub fn start(sensor_type: SensorType, delay_us: i32, cb: impl Fn(&SensorData) + 'static) {
    let type_id = sensor_type as i32;
    SENSOR_CALLBACKS.with(|map| {
        map.borrow_mut().insert(type_id, Box::new(cb));
    });
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

/// Stop receiving sensor data for the given type.
pub fn stop(sensor_type: SensorType) {
    let type_id = sensor_type as i32;
    SENSOR_CALLBACKS.with(|map| {
        map.borrow_mut().remove(&type_id);
    });
    bridge::with_activity(|env, activity| {
        if let Err(e) = env.call_method(activity, "stopSensor", "(I)V", &[JValue::Int(type_id)]) {
            log::warn!("stopSensor({type_id}) JNI call failed: {e}");
        }
    });
}

/// Drain latest sensor values and invoke registered callbacks.
/// Called from `android_runtime.rs` main loop each frame.
pub fn drain_sensor_events() {
    let snapshot: HashMap<i32, SensorData> = {
        let mut guard = SENSOR_DATA.lock().unwrap();
        guard.take().unwrap_or_default()
    };
    if snapshot.is_empty() {
        return;
    }
    SENSOR_CALLBACKS.with(|cbs| {
        let cbs = cbs.borrow();
        for (type_id, data) in &snapshot {
            if let Some(cb) = cbs.get(type_id) {
                cb(data);
            }
        }
    });
}

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
    SENSOR_DATA
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(
            sensor_type,
            SensorData {
                values: vals,
                num_values: num,
                timestamp_ns: timestamp as u64,
            },
        );
}
