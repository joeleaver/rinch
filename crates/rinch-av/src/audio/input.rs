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
    #[cfg(target_arch = "wasm32")]
    _wasm_state: Option<WasmAudioInputState>,
    config: AudioInputConfig,
}

#[cfg(target_arch = "wasm32")]
struct WasmAudioInputState {
    audio_ctx: web_sys::AudioContext,
    _source: web_sys::MediaStreamAudioSourceNode,
    processor: web_sys::ScriptProcessorNode,
    _closure: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
    tracks: Vec<web_sys::MediaStreamTrack>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WasmAudioInputState {
    fn drop(&mut self) {
        // Disconnect processor
        let _ = self.processor.disconnect();
        // Stop all tracks
        for track in &self.tracks {
            track.stop();
        }
        // Close audio context
        let _ = self.audio_ctx.close();
    }
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

/// Async version of audio input device enumeration for WASM.
#[cfg(target_arch = "wasm32")]
pub async fn audio_input_devices_async() -> Result<Vec<DeviceInfo>, AvError> {
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
        if info.kind() == web_sys::MediaDeviceKind::Audioinput {
            let device_id = info.device_id();
            let label = info.label();
            devices.push(DeviceInfo {
                id: DeviceId(if device_id.is_empty() {
                    format!("audioinput-{}", idx)
                } else {
                    device_id
                }),
                name: if label.is_empty() {
                    format!("Microphone {}", idx)
                } else {
                    label
                },
                kind: DeviceKind::AudioInput,
                is_default: idx == 0,
            });
            idx += 1;
        }
    }
    Ok(devices)
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

    #[cfg(target_arch = "wasm32")]
    {
        open_audio_input_wasm(None, config, on_data)
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
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

    #[cfg(target_arch = "wasm32")]
    {
        open_audio_input_wasm(Some(device.0.clone()), config, on_data)
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        let _ = (device, config, on_data);
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

#[cfg(target_arch = "wasm32")]
fn open_audio_input_wasm(
    device_id: Option<String>,
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<AudioInputStream, AvError> {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let md = web_sys::window()
        .ok_or_else(|| AvError::Backend("no window".into()))?
        .navigator()
        .media_devices()
        .map_err(|_| AvError::Backend("mediaDevices not available".into()))?;

    // Build audio constraints
    let audio_constraints = if let Some(ref id) = device_id {
        let obj = js_sys::Object::new();
        let exact = js_sys::Object::new();
        js_sys::Reflect::set(&exact, &"exact".into(), &JsValue::from_str(id)).unwrap();
        js_sys::Reflect::set(&obj, &"deviceId".into(), &exact).unwrap();
        JsValue::from(obj)
    } else {
        JsValue::TRUE
    };

    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&audio_constraints);
    constraints.set_video(&JsValue::FALSE);

    let promise = md
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| AvError::Backend(format!("getUserMedia failed: {:?}", e)))?;

    // We need to store the WASM state once the async operation completes.
    // Since the API is sync, we spawn the async work and store state in a shared cell.
    let state_cell: Rc<RefCell<Option<WasmAudioInputState>>> = Rc::new(RefCell::new(None));
    let state_clone = state_cell.clone();

    let buffer_size = if config.buffer_size == 0 {
        4096
    } else {
        config.buffer_size
    };
    let sample_rate = config.sample_rate;

    wasm_bindgen_futures::spawn_local(async move {
        let result = JsFuture::from(promise).await;
        let media_stream: web_sys::MediaStream = match result {
            Ok(val) => val.into(),
            Err(e) => {
                tracing::error!("getUserMedia rejected: {:?}", e);
                return;
            }
        };

        // Collect tracks for cleanup
        let track_list = media_stream.get_audio_tracks();
        let mut tracks = Vec::new();
        for i in 0..track_list.length() {
            let track: web_sys::MediaStreamTrack = track_list.get(i).into();
            tracks.push(track);
        }

        // Create AudioContext
        let ctx_opts = web_sys::AudioContextOptions::new();
        ctx_opts.set_sample_rate(sample_rate as f32);
        let audio_ctx = match web_sys::AudioContext::new_with_context_options(&ctx_opts) {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::error!("Failed to create AudioContext: {:?}", e);
                return;
            }
        };

        // Create source from the media stream
        let source = match audio_ctx.create_media_stream_source(&media_stream) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to create MediaStreamSource: {:?}", e);
                return;
            }
        };

        // Create ScriptProcessorNode
        let processor = match audio_ctx.create_script_processor_with_buffer_size(buffer_size) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to create ScriptProcessorNode: {:?}", e);
                return;
            }
        };

        // Set up the onaudioprocess handler
        let on_data = Rc::new(RefCell::new(on_data));
        let closure = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new(
            move |event: web_sys::AudioProcessingEvent| {
                let input_buffer = match event.input_buffer() {
                    Ok(buf) => buf,
                    Err(_) => return,
                };

                // Get channel 0 data
                if let Ok(channel_data) = input_buffer.get_channel_data(0) {
                    (on_data.borrow_mut())(channel_data.as_slice());
                }
            },
        );

        processor.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));

        // Connect: source -> processor -> destination
        let _ = source.connect_with_audio_node(&processor);
        let _ = processor.connect_with_audio_node(&audio_ctx.destination());

        // Store state
        *state_clone.borrow_mut() = Some(WasmAudioInputState {
            audio_ctx,
            _source: source,
            processor,
            _closure: closure,
            tracks,
        });
    });

    Ok(AudioInputStream {
        _wasm_state: None, // Will be populated asynchronously via the Rc<RefCell>
        config,
    })
}
