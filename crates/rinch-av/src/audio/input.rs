//! Audio input (microphone capture).
//!
//! Open an input stream with a callback that receives captured audio samples.
//! The callback runs on cpal's audio thread — avoid blocking or allocations.
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
/// Dropping this stops capture. The underlying cpal stream is kept alive
/// as long as this struct exists.
pub struct AudioInputStream {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    _stream: cpal::Stream,
    config: AudioInputConfig,
}

impl AudioInputStream {
    /// Returns the config used for this stream.
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

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Open the default audio input device.
///
/// `on_data` is called on the audio thread with each captured buffer of
/// interleaved f32 samples in the range [-1.0, 1.0].
///
/// **Audio thread safety:** The `on_data` callback runs on a dedicated audio
/// thread. Avoid blocking, heap allocation, or mutex locking inside it.
/// Use lock-free structures (atomic, ring buffer) to send data to the main
/// thread.
pub fn open_audio_input(
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<AudioInputStream, AvError> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let device = crate::native::cpal_audio::default_input_device()?;
        let stream = crate::native::cpal_audio::open_input_stream(
            &device,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            on_data,
        )?;
        Ok(AudioInputStream {
            _stream: stream,
            config,
        })
    }

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        let _ = (config, on_data);
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
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let dev = crate::native::cpal_audio::get_input_device(device)?;
        let stream = crate::native::cpal_audio::open_input_stream(
            &dev,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            on_data,
        )?;
        Ok(AudioInputStream {
            _stream: stream,
            config,
        })
    }

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        let _ = (device, config, on_data);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}
