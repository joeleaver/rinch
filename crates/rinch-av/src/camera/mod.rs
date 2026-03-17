//! Camera capture device access.

#[cfg(not(target_arch = "wasm32"))]
mod capture;
#[cfg(target_arch = "wasm32")]
mod capture_wasm;

mod format;

#[cfg(not(target_arch = "wasm32"))]
pub use capture::{CameraCapture, CameraConfig, camera_devices, open_camera, open_camera_on};
#[cfg(target_arch = "wasm32")]
pub use capture_wasm::{
    CameraCapture, CameraConfig, camera_devices, camera_devices_async, open_camera, open_camera_on,
};

pub use format::{PixelFormat, VideoFrame, convert_to_rgba};
