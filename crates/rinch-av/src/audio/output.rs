//! Audio output (speaker playback).
//!
//! Open an output stream with a callback that fills sample buffers.
//! The callback runs on cpal's audio thread — avoid blocking, allocations,
//! or locking inside it. Use lock-free data structures to feed audio data.
//!
//! # Example
//!
//! ```no_run
//! use rinch_av::audio::{AudioOutputConfig, open_audio_output};
//!
//! // Play a 440 Hz sine wave
//! let mut phase = 0.0f32;
//! let _stream = open_audio_output(AudioOutputConfig::default(), move |buf| {
//!     for frame in buf.chunks_mut(2) {
//!         let sample = (phase * 2.0 * std::f32::consts::PI).sin() * 0.25;
//!         phase += 440.0 / 48000.0;
//!         for s in frame.iter_mut() {
//!             *s = sample;
//!         }
//!     }
//! }).unwrap();
//! ```

use crate::device::{DeviceId, DeviceInfo};
use crate::error::AvError;

/// Configuration for an audio output stream.
#[derive(Debug, Clone)]
pub struct AudioOutputConfig {
    /// Sample rate in Hz. Default: 48000.
    pub sample_rate: u32,
    /// Number of channels. Default: 2 (stereo).
    pub channels: u16,
    /// Buffer size in frames. Default: 1024.
    /// Set to 0 to let the backend choose.
    pub buffer_size: u32,
}

impl Default for AudioOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            buffer_size: 1024,
        }
    }
}

/// An active audio output stream.
///
/// Dropping this stops playback. The underlying cpal stream is kept alive
/// as long as this struct exists.
pub struct AudioOutputStream {
    // The cpal stream — must be kept alive for audio to play.
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    _stream: cpal::Stream,
    config: AudioOutputConfig,
}

impl AudioOutputStream {
    /// Returns the config used for this stream.
    pub fn config(&self) -> &AudioOutputConfig {
        &self.config
    }
}

/// Enumerate available audio output devices.
pub fn audio_output_devices() -> Result<Vec<DeviceInfo>, AvError> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        crate::native::cpal_audio::enumerate_output_devices()
    }

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Open the default audio output device.
///
/// `fill` is called on the audio thread to fill each buffer with interleaved
/// f32 samples in the range [-1.0, 1.0]. The buffer length depends on the
/// configured buffer size and channel count.
///
/// **Audio thread safety:** The `fill` callback runs on a dedicated audio
/// thread. Avoid blocking, heap allocation, or mutex locking inside it.
/// Use lock-free structures (atomic, ring buffer) to communicate with the
/// main thread.
pub fn open_audio_output(
    config: AudioOutputConfig,
    fill: impl FnMut(&mut [f32]) + Send + 'static,
) -> Result<AudioOutputStream, AvError> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let device = crate::native::cpal_audio::default_output_device()?;
        let stream = crate::native::cpal_audio::open_output_stream(
            &device,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            fill,
        )?;
        Ok(AudioOutputStream {
            _stream: stream,
            config,
        })
    }

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        let _ = (config, fill);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Open a specific audio output device by ID.
///
/// See [`open_audio_output`] for callback requirements.
pub fn open_audio_output_on(
    device: &DeviceId,
    config: AudioOutputConfig,
    fill: impl FnMut(&mut [f32]) + Send + 'static,
) -> Result<AudioOutputStream, AvError> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let dev = crate::native::cpal_audio::get_output_device(device)?;
        let stream = crate::native::cpal_audio::open_output_stream(
            &dev,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            fill,
        )?;
        Ok(AudioOutputStream {
            _stream: stream,
            config,
        })
    }

    #[cfg(not(all(not(target_arch = "wasm32"), feature = "audio")))]
    {
        let _ = (device, config, fill);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}
