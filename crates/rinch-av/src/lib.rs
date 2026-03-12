//! Cross-platform audio/video device access for rinch.
//!
//! `rinch-av` provides primitives for camera capture, microphone input,
//! and audio output, designed to layer WebRTC on top later.
//!
//! # Features
//!
//! - `audio` — Audio input/output via cpal (desktop)
//! - `camera` — Camera capture via nokhwa (desktop)
//!
//! # Architecture
//!
//! Device access is callback-based: you open a device with a closure that
//! receives frames/samples. For camera display, capture the `SurfaceWriter`
//! from rinch in your closure. Audio callbacks run on dedicated threads.
//!
//! The Track/Stream abstraction (`AudioTrack`, `VideoTrack`, `MediaStream`)
//! provides a WebRTC-aligned pub/sub model on top of the raw device APIs.

mod device;
mod error;

#[cfg(feature = "audio")]
pub mod audio;

#[cfg(feature = "camera")]
pub mod camera;

pub mod stream;
pub mod track;

// Platform backends
#[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
mod native;

pub use device::{DeviceId, DeviceInfo, DeviceKind};
pub use error::AvError;
pub use track::{TrackHandle, TrackId, TrackKind, TrackState};

#[cfg(feature = "audio")]
pub use track::{AudioChunk, AudioTrack, AudioTrackReader};

#[cfg(feature = "camera")]
pub use track::{VideoPipe, VideoTrack};

pub use stream::MediaStream;

#[cfg(any(feature = "audio", feature = "camera"))]
pub use stream::{MediaConstraints, get_user_media};
