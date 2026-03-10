//! VideoTrack — latest-frame semantics for video frames.

use super::{TrackHandle, TrackId, TrackKind, TrackState, generate_track_id};
use crate::camera::VideoFrame;
use rinch_core::Signal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A video track with latest-frame semantics.
///
/// Unlike `AudioTrack` (which queues every chunk), `VideoTrack` stores only
/// the most recent frame. Slow consumers always see the latest frame rather
/// than falling behind.
pub struct VideoTrack {
    pub handle: TrackHandle,
    latest: Arc<Mutex<Option<VideoFrame>>>,
    _capture: Option<crate::camera::CameraCapture>,
}

/// A handle to a running `pipe_to` operation.
///
/// Dropping this stops the pipe thread.
pub struct VideoPipe {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for VideoPipe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

impl VideoTrack {
    /// Create a new video track with the given label.
    pub fn new(label: &str) -> Self {
        Self {
            handle: TrackHandle {
                id: TrackId(generate_track_id()),
                kind: TrackKind::Video,
                label: label.to_string(),
                enabled: Signal::new(true),
                state: Signal::new(TrackState::Live),
            },
            latest: Arc::new(Mutex::new(None)),
            _capture: None,
        }
    }

    /// Create a video track from the default camera.
    pub fn from_camera(config: crate::camera::CameraConfig) -> Result<Self, crate::AvError> {
        let mut track = Self::new("camera");
        let latest = track.latest.clone();

        let capture = crate::camera::open_camera(config, move |frame| {
            *latest.lock().unwrap() = Some(frame);
        })?;

        track._capture = Some(capture);
        Ok(track)
    }

    /// Create a video track from a specific camera device.
    pub fn from_camera_on(
        device: &crate::DeviceId,
        config: crate::camera::CameraConfig,
    ) -> Result<Self, crate::AvError> {
        let mut track = Self::new("camera");
        let latest = track.latest.clone();

        let capture = crate::camera::open_camera_on(device, config, move |frame| {
            *latest.lock().unwrap() = Some(frame);
        })?;

        track._capture = Some(capture);
        Ok(track)
    }

    /// Get the latest frame (non-blocking). Returns `None` if no frame has arrived yet.
    pub fn latest_frame(&self) -> Option<VideoFrame> {
        self.latest.lock().unwrap().clone()
    }

    /// Pump frames to a callback on a background thread.
    ///
    /// The callback receives each new frame as it arrives. Returns a `VideoPipe`
    /// handle — dropping it stops the pipe.
    pub fn pipe_to(&self, mut sink: impl FnMut(&VideoFrame) + Send + 'static) -> VideoPipe {
        let latest = self.latest.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let mut last_ts = 0u64;

        let thread = std::thread::Builder::new()
            .name("rinch-av-videopipe".into())
            .spawn(move || {
                while !stop_clone.load(Ordering::Relaxed) {
                    let frame = latest.lock().unwrap().clone();
                    if let Some(ref f) = frame
                        && f.timestamp_us != last_ts
                    {
                        last_ts = f.timestamp_us;
                        sink(f);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            })
            .expect("failed to spawn video pipe thread");

        VideoPipe {
            stop,
            thread: Some(thread),
        }
    }

    /// Store a frame manually (for non-device sources).
    pub fn push_frame(&self, frame: VideoFrame) {
        *self.latest.lock().unwrap() = Some(frame);
    }

    /// Get the track handle.
    pub fn track(&self) -> &TrackHandle {
        &self.handle
    }
}
