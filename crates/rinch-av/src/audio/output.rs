//! Audio output (speaker playback).
//!
//! Open an output stream with a callback that fills sample buffers.
//! The callback runs on cpal's audio thread (desktop) or ScriptProcessorNode
//! handler (WASM) — avoid blocking, allocations, or locking inside it.
//! Use lock-free data structures to feed audio data.
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
/// Dropping this stops playback.
pub struct AudioOutputStream {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    _stream: cpal::Stream,
    #[cfg(target_arch = "wasm32")]
    _wasm_state: Option<WasmAudioOutputState>,
    config: AudioOutputConfig,
}

#[cfg(target_arch = "wasm32")]
struct WasmAudioOutputState {
    audio_ctx: web_sys::AudioContext,
    processor: web_sys::ScriptProcessorNode,
    _closure: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WasmAudioOutputState {
    fn drop(&mut self) {
        let _ = self.processor.disconnect();
        let _ = self.audio_ctx.close();
    }
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

    #[cfg(target_arch = "wasm32")]
    {
        // Sync API returns empty on WASM. Use audio_output_devices_async() for full list.
        Ok(Vec::new())
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

/// Async version of audio output device enumeration for WASM.
#[cfg(target_arch = "wasm32")]
pub async fn audio_output_devices_async() -> Result<Vec<DeviceInfo>, AvError> {
    use crate::device::DeviceKind;
    use wasm_bindgen_futures::JsFuture;

    let md = web_sys::window()
        .ok_or_else(|| AvError::Backend("no window".into()))?
        .navigator()
        .media_devices()
        .map_err(|_| AvError::Backend("mediaDevices not available".into()))?;

    let promise = md
        .enumerate_devices()
        .map_err(|e| AvError::Backend(format!("enumerateDevices failed: {:?}", e)))?;
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| AvError::Backend(format!("enumerateDevices rejected: {:?}", e)))?;

    let array: js_sys::Array = result.into();
    let mut devices = Vec::new();
    let mut idx = 0u32;
    for item in array.iter() {
        let info: web_sys::MediaDeviceInfo = item.into();
        if info.kind() == web_sys::MediaDeviceKind::Audiooutput {
            let device_id = info.device_id();
            let label = info.label();
            devices.push(DeviceInfo {
                id: DeviceId(if device_id.is_empty() {
                    format!("audiooutput-{}", idx)
                } else {
                    device_id
                }),
                name: if label.is_empty() {
                    format!("Speaker {}", idx)
                } else {
                    label
                },
                kind: DeviceKind::AudioOutput,
                is_default: idx == 0,
            });
            idx += 1;
        }
    }
    Ok(devices)
}

/// Open the default audio output device.
///
/// `fill` is called to fill each buffer with interleaved f32 samples
/// in the range [-1.0, 1.0].
///
/// On WASM, this creates an `AudioContext` with a `ScriptProcessorNode`.
/// Must be called from a user gesture context (the AudioContext requires it).
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

    #[cfg(target_arch = "wasm32")]
    {
        open_audio_output_wasm(config, fill)
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
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

    #[cfg(target_arch = "wasm32")]
    {
        // On WASM, device selection for output is not well-supported by browsers.
        // We ignore the device ID and use the default output.
        let _ = device;
        open_audio_output_wasm(config, fill)
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        let _ = (device, config, fill);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

#[cfg(target_arch = "wasm32")]
fn open_audio_output_wasm(
    config: AudioOutputConfig,
    fill: impl FnMut(&mut [f32]) + Send + 'static,
) -> Result<AudioOutputStream, AvError> {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let buffer_size = if config.buffer_size == 0 {
        4096
    } else {
        config.buffer_size
    };

    // Create AudioContext
    let ctx_opts = web_sys::AudioContextOptions::new();
    ctx_opts.set_sample_rate(config.sample_rate as f32);
    let audio_ctx = web_sys::AudioContext::new_with_context_options(&ctx_opts)
        .map_err(|e| AvError::Backend(format!("Failed to create AudioContext: {:?}", e)))?;

    let channels = config.channels as u32;

    // Create ScriptProcessorNode for output
    let processor = audio_ctx
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
            buffer_size,
            0,        // no input channels
            channels, // output channels
        )
        .map_err(|e| {
            AvError::Backend(format!("Failed to create ScriptProcessorNode: {:?}", e))
        })?;

    // Set up the onaudioprocess handler
    let fill = Rc::new(RefCell::new(fill));
    let closure = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new({
        let fill = fill.clone();
        move |event: web_sys::AudioProcessingEvent| {
            let output_buffer = match event.output_buffer() {
                Ok(buf) => buf,
                Err(_) => return,
            };

            let length = output_buffer.length() as usize;
            let num_channels = output_buffer.number_of_channels() as usize;

            // Create interleaved buffer for the fill callback
            let total_samples = length * num_channels;
            let mut interleaved = vec![0.0f32; total_samples];

            (fill.borrow_mut())(&mut interleaved);

            // De-interleave into per-channel output buffers
            for ch in 0..num_channels {
                if let Ok(mut channel_data) = output_buffer.get_channel_data(ch as u32) {
                    for i in 0..length {
                        channel_data[i] = interleaved[i * num_channels + ch];
                    }
                    let _ = output_buffer.copy_to_channel(&channel_data, ch as i32);
                }
            }
        }
    });

    processor.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));

    // Connect processor to destination (speakers)
    let _ = processor.connect_with_audio_node(&audio_ctx.destination());

    let state = WasmAudioOutputState {
        audio_ctx,
        processor,
        _closure: closure,
    };

    Ok(AudioOutputStream {
        _wasm_state: Some(state),
        config,
    })
}
