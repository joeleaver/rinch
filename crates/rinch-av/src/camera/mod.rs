//! Camera capture device access.

mod capture;
mod format;

pub use capture::{CameraCapture, CameraConfig, camera_devices, open_camera, open_camera_on};
pub use format::{PixelFormat, VideoFrame, convert_to_rgba};
