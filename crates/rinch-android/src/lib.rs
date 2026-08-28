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
//! spans, not intra-doc links: every module here is
//! `#[cfg(target_os = "android")]`, so the paths do not resolve when the docs
//! are built on the host, which is where CI builds them.)

#[cfg(target_os = "android")]
mod bridge;
#[cfg(target_os = "android")]
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
#[cfg(target_os = "android")]
pub mod lifecycle;
#[cfg(target_os = "android")]
pub mod location;
#[cfg(target_os = "android")]
pub mod notification;
#[cfg(target_os = "android")]
pub mod permissions;
#[cfg(target_os = "android")]
pub mod sensors;
#[cfg(target_os = "android")]
pub mod share;

#[cfg(target_os = "android")]
pub use bridge::init;

#[cfg(not(target_os = "android"))]
pub fn init(_: &()) {}
