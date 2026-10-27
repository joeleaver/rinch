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
    /// Shared with the setup task, which fills it once getUserMedia (or the
    /// given stream) is ready. Dropping the stream drops the state, which
    /// stops the tracks; a setup that finishes after that drop stops them
    /// itself (it holds only a `Weak`).
    #[cfg(target_arch = "wasm32")]
    _wasm_state: std::rc::Rc<std::cell::RefCell<Option<WasmAudioInputState>>>,
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
    /// Samples are being delivered.
    Running,
    /// Web: the browser's autoplay policy holds the capture `AudioContext`
    /// suspended, so no samples arrive (the track itself is live). Capture
    /// resumes by itself on the page's next click, tap or key press. The
    /// usual way to never see this is to open capture from a click handler.
    WaitingForGesture,
    /// Setup failed. On the web the stream's tracks have been stopped and
    /// nothing is delivered.
    Failed(AvError),
}

#[cfg(target_arch = "wasm32")]
type JsHandler = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;

#[cfg(target_arch = "wasm32")]
struct WasmAudioInputState {
    audio_ctx: web_sys::AudioContext,
    _source: web_sys::MediaStreamAudioSourceNode,
    processor: web_sys::ScriptProcessorNode,
    _closure: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
    tracks: Vec<web_sys::MediaStreamTrack>,
    /// The context's `statechange` handler, keeping the status current.
    _on_state: JsHandler,
    /// While the context waits for a gesture: the window listener that
    /// resumes it on the next click, tap or key press.
    gesture: Option<JsHandler>,
}

#[cfg(target_arch = "wasm32")]
const GESTURES: [&str; 3] = ["pointerdown", "keydown", "touchend"];

#[cfg(target_arch = "wasm32")]
fn remove_gesture_listener(cb: &JsHandler) {
    use wasm_bindgen::JsCast;
    if let Some(w) = web_sys::window() {
        for ev in GESTURES {
            let _ = w.remove_event_listener_with_callback_and_bool(
                ev,
                cb.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for WasmAudioInputState {
    fn drop(&mut self) {
        // Detach the handler first: an audioprocess event already queued
        // would otherwise reach the closure after it is freed ("closure
        // invoked recursively or after being dropped").
        self.processor.set_onaudioprocess(None);
        self.audio_ctx.set_onstatechange(None);
        if let Some(g) = &self.gesture {
            remove_gesture_listener(g);
        }
        let _ = self.processor.disconnect();
        for track in &self.tracks {
            track.stop();
        }
        quiet(self.audio_ctx.close());
    }
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
        open_audio_input_wasm(device.map(|d| d.0.clone()), config, make(&actual))
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

/// Capture from a `MediaStream` the app already has: a remote WebRTC
/// track, an `OscillatorNode` through a `MediaStreamAudioDestinationNode`,
/// or a stream from `getUserMedia` it made itself. Web only. Dropping the
/// returned stream stops the stream's audio tracks.
#[cfg(target_arch = "wasm32")]
pub fn open_audio_input_from_stream(
    stream: &web_sys::MediaStream,
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> AudioInputStream {
    start_audio_input_wasm(
        js_sys::Promise::resolve(AsRef::<wasm_bindgen::JsValue>::as_ref(stream)),
        config,
        on_data,
    )
}

#[cfg(target_arch = "wasm32")]
fn open_audio_input_wasm(
    device_id: Option<String>,
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> Result<AudioInputStream, AvError> {
    use wasm_bindgen::prelude::*;

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
    Ok(start_audio_input_wasm(promise, config, on_data))
}

/// Set up capture once `stream_promise` resolves to a `MediaStream`.
#[cfg(target_arch = "wasm32")]
fn start_audio_input_wasm(
    stream_promise: js_sys::Promise,
    config: AudioInputConfig,
    on_data: impl FnMut(&[f32]) + Send + 'static,
) -> AudioInputStream {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    let state: Rc<RefCell<Option<WasmAudioInputState>>> = Rc::new(RefCell::new(None));
    let status = Rc::new(RefCell::new(AudioInputStatus::Starting));
    // The task holds only Weaks: the AudioInputStream owns the state.
    let weak = Rc::downgrade(&state);
    let weak_status = Rc::downgrade(&status);

    let buffer_size = if config.buffer_size == 0 {
        4096
    } else {
        config.buffer_size
    };
    let sample_rate = config.sample_rate;
    let channels = config.channels.max(1);

    wasm_bindgen_futures::spawn_local(async move {
        let set_status = |s: AudioInputStatus| {
            if let Some(st) = weak_status.upgrade() {
                *st.borrow_mut() = s;
            }
        };
        let media_stream: web_sys::MediaStream = match JsFuture::from(stream_promise).await {
            Ok(val) => val.into(),
            Err(e) => {
                tracing::error!("getUserMedia rejected: {:?}", e);
                set_status(AudioInputStatus::Failed(gum_error(&e)));
                return;
            }
        };

        let track_list = media_stream.get_audio_tracks();
        let tracks: Vec<web_sys::MediaStreamTrack> = (0..track_list.length())
            .map(|i| track_list.get(i).into())
            .collect();

        // The stream was dropped (stopped) before the setup finished: release
        // the device now instead of leaving it open.
        let Some(state) = weak.upgrade() else {
            for t in &tracks {
                t.stop();
            }
            return;
        };

        // Every failure from here on releases the device and says why: a
        // setup that only logged left a real microphone (and its indicator)
        // on, delivering nothing.
        let fail = |what: &str, e: JsValue, ctx: Option<&web_sys::AudioContext>| {
            tracing::error!("rinch-av: audio capture setup failed: {what}: {:?}", e);
            for t in &tracks {
                t.stop();
            }
            if let Some(c) = ctx {
                quiet(c.close());
            }
            set_status(AudioInputStatus::Failed(AvError::Backend(format!(
                "{what}: {}",
                js_message(&e)
            ))));
        };

        let ctx_opts = web_sys::AudioContextOptions::new();
        ctx_opts.set_sample_rate(sample_rate as f32);
        let audio_ctx = match web_sys::AudioContext::new_with_context_options(&ctx_opts) {
            Ok(ctx) => ctx,
            Err(e) => return fail("AudioContext", e, None),
        };
        let source = match audio_ctx.create_media_stream_source(&media_stream) {
            Ok(s) => s,
            Err(e) => return fail("MediaStreamAudioSourceNode", e, Some(&audio_ctx)),
        };
        let processor = match audio_ctx
            .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                buffer_size,
                channels as u32,
                channels as u32,
            ) {
            Ok(p) => p,
            Err(e) => return fail("ScriptProcessorNode", e, Some(&audio_ctx)),
        };

        // Interleave every channel, as on desktop (it used to deliver
        // channel 0 only, whatever `channels` said).
        let on_data = Rc::new(RefCell::new(on_data));
        let mut interleaved: Vec<f32> = Vec::new();
        let closure = Closure::<dyn FnMut(web_sys::AudioProcessingEvent)>::new(
            move |event: web_sys::AudioProcessingEvent| {
                let Ok(input) = event.input_buffer() else {
                    return;
                };
                let n = input.number_of_channels().min(channels as u32) as usize;
                let planes: Vec<Vec<f32>> = (0..n)
                    .filter_map(|c| input.get_channel_data(c as u32).ok())
                    .collect();
                if planes.is_empty() {
                    return;
                }
                let frames = planes[0].len();
                let last = planes.len() - 1;
                interleaved.clear();
                interleaved.extend((0..frames).flat_map(|i| {
                    let planes = &planes;
                    (0..channels as usize).map(move |c| planes[c.min(last)][i])
                }));
                (on_data.borrow_mut())(&interleaved);
            },
        );
        processor.set_onaudioprocess(Some(closure.as_ref().unchecked_ref()));
        if let Err(e) = source.connect_with_audio_node(&processor) {
            processor.set_onaudioprocess(None);
            return fail("connecting the source", e, Some(&audio_ctx));
        }
        if let Err(e) = processor.connect_with_audio_node(&audio_ctx.destination()) {
            processor.set_onaudioprocess(None);
            return fail("connecting the processor", e, Some(&audio_ctx));
        }

        // A context made outside a user gesture starts `suspended` and
        // delivers nothing, with no error. Ask it to run; if the browser
        // holds it, say so, and resume on the page's next gesture.
        let running = |c: &web_sys::AudioContext| c.state() == web_sys::AudioContextState::Running;
        let ctx2 = audio_ctx.clone();
        let ws = weak_status.clone();
        let weak_state = Rc::downgrade(&state);
        let on_state = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            let Some(st) = ws.upgrade() else { return };
            let now = match ctx2.state() {
                web_sys::AudioContextState::Running => AudioInputStatus::Running,
                web_sys::AudioContextState::Suspended => AudioInputStatus::WaitingForGesture,
                _ => return,
            };
            let run = now == AudioInputStatus::Running;
            *st.borrow_mut() = now;
            if run {
                // Running: the gesture listener has done its job. Dropped
                // after this call (it may be on the stack).
                if let Some(s) = weak_state.upgrade() {
                    let g = s.borrow_mut().as_mut().and_then(|s| s.gesture.take());
                    if let Some(g) = g {
                        remove_gesture_listener(&g);
                        wasm_bindgen_futures::spawn_local(async move { drop(g) });
                    }
                }
            }
        });
        audio_ctx.set_onstatechange(Some(on_state.as_ref().unchecked_ref()));
        quiet(audio_ctx.resume());
        let gesture = if running(&audio_ctx) {
            set_status(AudioInputStatus::Running);
            None
        } else {
            set_status(AudioInputStatus::WaitingForGesture);
            tracing::warn!(
                "rinch-av: the browser holds audio capture until the page gets a click or key \
                 press (autoplay policy); no samples until then"
            );
            let ctx3 = audio_ctx.clone();
            let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                quiet(ctx3.resume());
            });
            if let Some(w) = web_sys::window() {
                for ev in GESTURES {
                    let _ = w.add_event_listener_with_callback_and_bool(
                        ev,
                        cb.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
            Some(cb)
        };

        *state.borrow_mut() = Some(WasmAudioInputState {
            audio_ctx,
            _source: source,
            processor,
            _closure: closure,
            tracks,
            _on_state: on_state,
            gesture,
        });
    });

    AudioInputStream {
        _wasm_state: state,
        status,
        config,
    }
}

/// Let a promise settle with nothing listening, without the browser
/// reporting an unhandled rejection (a `resume()` the page's `close()`
/// overtook rejects "Closed before resume completed").
#[cfg(target_arch = "wasm32")]
fn quiet(p: Result<js_sys::Promise, wasm_bindgen::JsValue>) {
    if let Ok(p) = p {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(p).await;
        });
    }
}

/// The `name`/`message` of a JS error.
#[cfg(target_arch = "wasm32")]
fn js_message(e: &wasm_bindgen::JsValue) -> String {
    let get = |k: &str| {
        js_sys::Reflect::get(e, &k.into())
            .ok()
            .and_then(|v| v.as_string())
    };
    match (get("name"), get("message")) {
        (Some(n), Some(m)) => format!("{n}: {m}"),
        (None, Some(m)) => m,
        _ => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

/// A `getUserMedia` rejection as an `AvError`.
#[cfg(target_arch = "wasm32")]
fn gum_error(e: &wasm_bindgen::JsValue) -> AvError {
    let name = js_sys::Reflect::get(e, &"name".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    match name.as_str() {
        "NotAllowedError" | "SecurityError" => AvError::PermissionDenied,
        "NotFoundError" | "OverconstrainedError" => AvError::DeviceNotFound(js_message(e)),
        "NotReadableError" | "AbortError" => AvError::DeviceInUse,
        _ => AvError::Backend(js_message(e)),
    }
}
