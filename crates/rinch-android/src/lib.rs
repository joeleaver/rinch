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

#[cfg(target_os = "android")]
mod bridge;
#[cfg(target_os = "android")]
pub mod callback;
#[cfg(target_os = "android")]
pub mod clipboard;
#[cfg(target_os = "android")]
pub mod display;
#[cfg(target_os = "android")]
pub mod file_picker;
#[cfg(target_os = "android")]
pub mod ime;
#[cfg(target_os = "android")]
pub mod permissions;

#[cfg(target_os = "android")]
pub use bridge::init;

#[cfg(not(target_os = "android"))]
pub fn init(_: &()) {}
