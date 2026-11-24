//! The web backend of audio input: a `ScriptProcessorNode` on an
//! `AudioContext` fed by a `MediaStream` (from `getUserMedia`, or one the app
//! gives), and the status that says whether samples arrive.

use super::{AudioInputConfig, AudioInputStatus, AudioInputStream};
use crate::device::{DeviceId, DeviceInfo};
use crate::error::AvError;

type JsHandler = wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>;

pub(super) struct WasmAudioInputState {
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

const GESTURES: [&str; 3] = ["pointerdown", "keydown", "touchend"];

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

/// Async version of audio input device enumeration for WASM.
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

/// Capture from a `MediaStream` the app already has: a remote WebRTC
/// track, an `OscillatorNode` through a `MediaStreamAudioDestinationNode`,
/// or a stream from `getUserMedia` it made itself. Web only. Dropping the
/// returned stream stops the stream's audio tracks.
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

pub(super) fn open_audio_input_wasm(
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
fn quiet(p: Result<js_sys::Promise, wasm_bindgen::JsValue>) {
    if let Ok(p) = p {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(p).await;
        });
    }
}

/// The `name`/`message` of a JS error.
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
