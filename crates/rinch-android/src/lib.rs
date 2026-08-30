//! Android platform bridge for rinch.
//!
//! Provides JNI infrastructure for calling Android platform APIs from Rust.
//! Each module wraps a specific platform service (clipboard, IME, etc.)
//! and calls methods on `RinchActivity.java` via JNI.
//!
//! # Initialization
//!
//! Call [`init`] once from `android_main` before using any service:
//!
//! ```ignore
//! rinch_android::init(&android_app);
//! ```
//!
//! # Minimum API level
//!
//! This crate assumes **API 28** (Android 9). Nothing declares it — the floor
//! comes from `examples/hello-android`'s `AndroidManifest.xml` and
//! `build-apk.sh`, both of which say `minSdk 28` — so a downstream app is free
//! to ship lower. Below 28 the calls that branch on `Build.VERSION.SDK_INT`
//! take their oldest path or become no-ops rather than failing:
//! `display::set_light_status_bars` and `display::set_light_navigation_bars`
//! write system-UI visibility flags that API 21–25 frameworks simply ignore, so
//! the bars keep the system's default light-on-dark contents. (Plain code
//! spans, not intra-doc links: those modules are
//! `#[cfg(target_os = "android")]`, so the paths do not resolve when the docs
//! are built on the host, which is where CI builds them.)
//!
//! # What compiles on the host
//!
//! Most modules here are `#[cfg(target_os = "android")]` in their entirety: they
//! are JNI wrappers with nothing to say off-device. The four that hold a
//! **callback registry** — [`callback`], [`lifecycle`], [`location`],
//! [`sensors`] — are not. Their registries, drains and scope-lifetime rules (see
//! `scoped`) compile and are unit-tested on every CI machine, and only the JNI
//! half of each operation sits behind the `cfg`, as a small private function
//! that is a no-op off-device. Nothing in this repository builds an APK in CI,
//! so logic left behind the `cfg` is logic nothing checks (issue #183 PR5).

#[cfg(target_os = "android")]
mod bridge;
pub mod callback;
#[cfg(target_os = "android")]
pub mod camera;
#[cfg(target_os = "android")]
pub mod clipboard;
#[cfg(target_os = "android")]
pub mod display;
#[cfg(target_os = "android")]
pub mod file_picker;
#[cfg(target_os = "android")]
pub mod ime;
pub mod lifecycle;
pub mod location;
#[cfg(target_os = "android")]
pub mod notification;
#[cfg(target_os = "android")]
pub mod permissions;
pub(crate) mod scoped;
pub mod sensors;
#[cfg(target_os = "android")]
pub mod share;

#[cfg(target_os = "android")]
pub use bridge::init;

#[cfg(not(target_os = "android"))]
pub fn init(_: &()) {}

/// Serialisation for tests that drive a drain function.
///
/// The callback registries are `thread_local!`, and `cargo test` gives each test
/// its own thread, so those isolate for free. The *event queues* they drain do
/// not: `SENSOR_DATA`, `LOCATION`, `PENDING_EVENT`, `ACTIVITY_RESULTS` and
/// `PERMISSION_RESULTS` are process-global statics, and every drain empties its
/// queue — so one test's drain will happily swallow another test's queued event
/// and report "nothing happened" to both. Tests that queue-then-drain take this
/// lock; tests that only touch the thread-local registries do not need it.
#[cfg(test)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // Poison-tolerant: a failing test panics while holding the guard, and the
    // remaining tests are still worth running.
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
