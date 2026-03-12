use crate::AvError;
use crate::camera::format::{PixelFormat, VideoFrame};
use crate::device::{DeviceId, DeviceInfo, DeviceKind};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

/// Configuration for camera capture.
#[derive(Debug, Clone)]
pub struct CameraConfig {
    /// Requested width in pixels.
    pub width: u32,
    /// Requested height in pixels.
    pub height: u32,
    /// Requested frames per second.
    pub fps: u32,
    /// Preferred pixel format. `None` lets the backend choose.
    pub format: Option<PixelFormat>,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
            fps: 30,
            format: None,
        }
    }
}

/// An active camera capture session.
///
/// Frames are delivered via the callback passed to [`open_camera`] or [`open_camera_on`].
/// Dropping this struct stops the capture thread.
pub struct CameraCapture {
    stop_flag: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CameraCapture {
    /// Stop capture and wait for the thread to finish.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Enumerate available camera devices.
pub fn camera_devices() -> Result<Vec<DeviceInfo>, AvError> {
    let cameras = nokhwa::query(ApiBackend::Auto).map_err(map_nokhwa_error)?;
    Ok(cameras
        .into_iter()
        .enumerate()
        .map(|(i, info)| DeviceInfo {
            id: DeviceId(format!("{}", i)),
            name: info.human_name(),
            kind: DeviceKind::Camera,
            is_default: i == 0,
        })
        .collect())
}

/// Open the default camera.
///
/// `on_frame` is called on a background capture thread with each decoded RGBA frame.
pub fn open_camera(
    config: CameraConfig,
    on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    open_camera_with_index(CameraIndex::Index(0), config, on_frame)
}

/// Open a specific camera by device ID.
///
/// `on_frame` is called on a background capture thread with each decoded RGBA frame.
pub fn open_camera_on(
    device: &DeviceId,
    config: CameraConfig,
    on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    let index = match device.0.parse::<u32>() {
        Ok(idx) => CameraIndex::Index(idx),
        Err(_) => CameraIndex::String(device.0.clone()),
    };
    open_camera_with_index(index, config, on_frame)
}

fn pixel_format_to_frame_format(format: PixelFormat) -> FrameFormat {
    match format {
        PixelFormat::Rgba => FrameFormat::RAWRGB,
        PixelFormat::Yuyv => FrameFormat::YUYV,
        PixelFormat::Nv12 => FrameFormat::NV12,
        PixelFormat::Mjpeg => FrameFormat::MJPEG,
    }
}

fn build_requested_format(config: &CameraConfig) -> RequestedFormat<'static> {
    let ff = config
        .format
        .map(pixel_format_to_frame_format)
        .unwrap_or(FrameFormat::MJPEG);
    let camera_fmt = CameraFormat::new_from(config.width, config.height, ff, config.fps);
    RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(camera_fmt))
}

fn open_camera_with_index(
    index: CameraIndex,
    config: CameraConfig,
    mut on_frame: impl FnMut(VideoFrame) + Send + 'static,
) -> Result<CameraCapture, AvError> {
    let requested = build_requested_format(&config);

    // nokhwa::Camera is !Send, so we must create it on the capture thread.
    // Use a channel to report initialization success/failure back to the caller.
    let (init_tx, init_rx) = mpsc::channel::<Result<(), AvError>>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_clone = stop_flag.clone();

    let thread = std::thread::Builder::new()
        .name("rinch-av-camera".into())
        .spawn(move || {
            let mut camera = match nokhwa::Camera::new(index, requested) {
                Ok(c) => c,
                Err(e) => {
                    let _ = init_tx.send(Err(map_nokhwa_error(e)));
                    return;
                }
            };

            if let Err(e) = camera.open_stream() {
                let _ = init_tx.send(Err(map_nokhwa_error(e)));
                return;
            }

            // Signal success to the caller.
            let _ = init_tx.send(Ok(()));

            let start = Instant::now();

            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }

                match camera.frame() {
                    Ok(buffer) => {
                        let res = buffer.resolution();
                        let w = res.width_x;
                        let h = res.height_y;

                        let rgba_data = match buffer.decode_image::<RgbAFormat>() {
                            Ok(img) => img.into_raw(),
                            Err(e) => {
                                tracing::warn!("Frame decode error: {}", e);
                                continue;
                            }
                        };

                        let frame = VideoFrame {
                            data: rgba_data,
                            width: w,
                            height: h,
                            timestamp_us: start.elapsed().as_micros() as u64,
                        };

                        on_frame(frame);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("disconnected")
                            || msg.contains("Device not found")
                            || msg.contains("VIDIOC_DQBUF")
                        {
                            tracing::error!("Camera disconnected: {}", e);
                            break;
                        }
                        tracing::warn!("Frame capture error: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            }

            let _ = camera.stop_stream();
        })
        .map_err(|e| AvError::Backend(format!("Failed to spawn capture thread: {}", e)))?;

    // Wait for the camera to initialize on the capture thread.
    match init_rx.recv() {
        Ok(Ok(())) => Ok(CameraCapture {
            stop_flag,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err(AvError::Backend(
                "Camera thread exited before initialization".into(),
            ))
        }
    }
}

fn map_nokhwa_error(e: nokhwa::NokhwaError) -> AvError {
    let msg = e.to_string();
    if msg.contains("Permission") || msg.contains("permission") {
        AvError::PermissionDenied
    } else if msg.contains("not found") || msg.contains("NotFound") {
        AvError::DeviceNotFound(msg)
    } else if msg.contains("busy") || msg.contains("in use") || msg.contains("EBUSY") {
        AvError::DeviceInUse
    } else {
        AvError::Backend(msg)
    }
}
