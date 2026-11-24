//! Audio input (microphone capture).
//!
//! Open an input stream with a callback that receives captured audio samples.
//! The callback runs on cpal's audio thread (desktop) or ScriptProcessorNode
//! handler (WASM) — avoid blocking or allocations.
//!
//! # Example
//!
//! ```no_run
//! use rinch_av::audio::{AudioInputConfig, open_audio_input};
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicU32, Ordering};
//!
//! // Capture microphone audio and compute RMS level
//! let level = Arc::new(AtomicU32::new(0));
//! let level_writer = level.clone();
//!
//! let _stream = open_audio_input(AudioInputConfig::default(), move |samples| {
//!     let rms = (samples.iter().map(|s| s * s).sum::<f32>()
//!         / samples.len() as f32).sqrt();
//!     level_writer.store(rms.to_bits(), Ordering::Relaxed);
//! }).unwrap();
//! ```

use crate::device::{DeviceId, DeviceInfo};
use crate::error::AvError;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::{audio_input_devices_async, open_audio_input_from_stream};

/// Configuration for an audio input stream.
#[derive(Debug, Clone)]
pub struct AudioInputConfig {
    /// Sample rate in Hz. Default: 48000.
    pub sample_rate: u32,
    /// Number of channels. Default: 1 (mono).
    pub channels: u16,
    /// Buffer size in frames. Default: 1024.
    /// Set to 0 to let the backend choose.
    pub buffer_size: u32,
}

impl Default for AudioInputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            buffer_size: 1024,
        }
    }
}

/// An active audio input stream.
///
/// Dropping this stops capture. The underlying cpal stream (desktop) or
/// AudioContext (WASM) is kept alive as long as this struct exists.
pub struct AudioInputStream {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    _stream: cpal::Stream,
    /// Shared with the setup task, which fills it once getUserMedia (or the
    /// given stream) is ready. Dropping the stream drops the state, which
    /// stops the tracks; a setup that finishes after that drop stops them
    /// itself (it holds only a `Weak`).
    #[cfg(target_arch = "wasm32")]
    _wasm_state: std::rc::Rc<std::cell::RefCell<Option<web::WasmAudioInputState>>>,
    /// Where the web setup stands (it finishes after the open call returns).
    #[cfg(target_arch = "wasm32")]
    status: std::rc::Rc<std::cell::RefCell<AudioInputStatus>>,
    config: AudioInputConfig,
}

/// Whether an [`AudioInputStream`] is delivering samples.
///
/// On desktop a stream that opened is `Running`: the device is open when
/// the open call returns. On the web the setup finishes later (after
/// `getUserMedia`, or the given stream, resolves), and the browser may hold
/// the capture `AudioContext` until the user interacts with the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioInputStatus {
    /// Web: setup has not finished yet.
    Starting,
    /// Samples are being delivered. On the web a delivered buffer makes it
    /// `Running` whatever the context's `state` says: Firefox delivers a
    /// context's first buffers, and its first after a resume, before
    /// `state` reads `running` (#3007). The one exception is a buffer the
    /// context queued before it stopped, arriving after the stop.
    Running,
    /// Web: the browser's autoplay policy holds the capture `AudioContext`
    /// suspended, so no samples arrive (the track itself is live), or a
    /// context that ran has stopped (suspended, or interrupted: Safari's
    /// state while another app or a call holds the audio session). Capture
    /// resumes by itself on the page's next click, tap or key press, or
    /// when the browser resumes the context (an interruption ending). The
    /// usual way to never see this is to open capture from a click handler.
    WaitingForGesture,
    /// Setup failed, or (web) the capture `AudioContext` was closed under
    /// it. On the web the stream's tracks have been stopped and nothing is
    /// delivered.
    Failed(AvError),
}

impl AudioInputStream {
    /// Whether the stream is delivering samples (see [`AudioInputStatus`]).
    /// Always `Running` on desktop.
    pub fn status(&self) -> AudioInputStatus {
        #[cfg(target_arch = "wasm32")]
        {
            self.status.borrow().clone()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            AudioInputStatus::Running
        }
    }

    /// What the stream really delivers: the sample rate and channel count
    /// the device was opened with, which may differ from what was requested
    /// (the nearest the device supports). Samples are interleaved by this
    /// channel count.
    pub fn config(&self) -> &AudioInputConfig {
        &self.config
    }
}

/// Enumerate available audio input devices.
pub fn audio_input_devices() -> Result<Vec<DeviceInfo>, AvError> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        crate::native::cpal_audio::enumerate_input_devices()
    }

    #[cfg(target_arch = "wasm32")]
    {
        // Sync API returns empty on WASM. Use audio_input_devices_async() for full list.
        Ok(Vec::new())
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Open the default audio input device.
///
/// `on_data` is called with each captured buffer of interleaved f32 samples
/// in the range [-1.0, 1.0].
///
/// On WASM, this calls `getUserMedia({audio: true})` and uses a
/// `ScriptProcessorNode` to extract samples. Must be called from a user
/// gesture context.
pub fn open_audio_input(
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<AudioInputStream, AvError> {
    open_audio_input_with(None, config, move |_| on_data)
}

/// Open an input with a callback built once the stream's real format is
/// known (`make` gets the negotiated config), so a consumer that tags its
/// samples (an `AudioTrack`) tags them with what the device really runs.
pub(crate) fn open_audio_input_with<F: FnMut(&[f32]) + Send + 'static>(
    device: Option<&DeviceId>,
    config: AudioInputConfig,
    make: impl FnOnce(&AudioInputConfig) -> F,
) -> Result<AudioInputStream, AvError> {
    crate::testing::gate("microphone")?;
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let dev = match device {
            Some(id) => crate::native::cpal_audio::get_input_device(id)?,
            None => crate::native::cpal_audio::default_input_device()?,
        };
        let buffer_size = config.buffer_size;
        let (stream, n) = crate::native::cpal_audio::open_input_stream(
            &dev,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            |n| {
                make(&AudioInputConfig {
                    sample_rate: n.sample_rate,
                    channels: n.channels,
                    buffer_size,
                })
            },
        )?;
        Ok(AudioInputStream {
            _stream: stream,
            config: AudioInputConfig {
                sample_rate: n.sample_rate,
                channels: n.channels,
                buffer_size,
            },
        })
    }

    #[cfg(target_arch = "wasm32")]
    {
        let actual = config.clone();
        web::open_audio_input_wasm(device.map(|d| d.0.clone()), config, make(&actual))
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        let _ = (device, config, make);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Open a specific audio input device by ID.
///
/// See [`open_audio_input`] for callback requirements.
pub fn open_audio_input_on(
    device: &DeviceId,
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<AudioInputStream, AvError> {
    open_audio_input_with(Some(device), config, move |_| on_data)
}
