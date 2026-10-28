//! Web media backend: the browser's own tracks, senders and transceivers
//! (design record `docs/design/rtc-media.md`, section 2).
//!
//! No frame passes through Rust on the way out or in: a camera is a
//! `getUserMedia` track, an app-fed source is a hidden canvas's
//! `captureStream` (video) or an `AudioContext` feeding a
//! `MediaStreamAudioDestinationNode` (audio), and a remote track plays in a
//! `<video>` ([`MediaView`](crate::media::MediaView)) or a hidden `<audio>`.
//!
//! Everything runs on the one web thread. The JS closures are owned here and
//! detached before they drop, as in `wasm.rs`.
//!
//! **`MediaStreamTrack` and `MediaStream` have an inherent `clone`**, the
//! JS `clone()` that makes a *new* track (or stream). It outranks
//! `Clone::clone`, so `track.clone()` here would hand a sender a copy that
//! `set_enabled` and `stop` never reach. Write `Clone::clone(&track)`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::atomic::Ordering;

use js_sys::{Array, Object, Reflect};
use rinch_core::dom::{NodeHandle, RenderScope};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioProcessingEvent, CanvasRenderingContext2d,
    HtmlAudioElement, HtmlCanvasElement, HtmlVideoElement, MediaDeviceInfo, MediaDeviceKind,
    MediaStream, MediaStreamAudioDestinationNode, MediaStreamConstraints, MediaStreamTrack,
    MediaTrackConstraints, RtcPeerConnection, RtcRtpSender, RtcRtpTransceiver,
    RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcTrackEvent, ScriptProcessorNode,
};

use crate::RtcError;
use crate::media::{
    self, AudioQueue, AudioSourceOptions, AudioSourceWriter, CameraOptions, DeviceInfo, Direction,
    MediaError, MediaEvent, MediaKind, MediaTrack, MicrophoneOptions, VideoSourceOptions,
    VideoSourceWriter,
};
use crate::registry::{self, Event};

type JsCallback = Closure<dyn FnMut(JsValue)>;

fn js_err(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn document() -> Option<web_sys::Document> {
    window()?.document()
}

/// Call `obj[name](args..)`, catching a throw (web-sys's bindings for
/// `addTrack`, `addTransceiver` and friends do not).
fn call(obj: &JsValue, name: &str, args: &[&JsValue]) -> Result<JsValue, JsValue> {
    let f: js_sys::Function = Reflect::get(obj, &name.into())?.dyn_into()?;
    let a: Array = args.iter().copied().collect();
    f.apply(obj, &a)
}

/// A running frame loop. [`stop`](Self::stop) breaks it from outside: a
/// loop whose callback is never called again (a video that presents no more
/// frames) would otherwise hold its closure, and everything it captured,
/// for ever, since the closure holds itself to re-register.
struct Loop(Box<dyn Fn()>);

impl Loop {
    fn stop(&self) {
        (self.0)()
    }
}

/// The slot a self-re-registering closure lives in, and the stopper for it.
fn loop_slot<F: ?Sized + 'static>() -> (Rc<RefCell<Option<Closure<F>>>>, Loop) {
    let slot: Rc<RefCell<Option<Closure<F>>>> = Rc::new(RefCell::new(None));
    let weak = Rc::downgrade(&slot);
    let stop = Loop(Box::new(move || {
        if let Some(s) = weak.upgrade() {
            // Dropped after the current call: it may be the caller.
            let gone = s.borrow_mut().take();
            wasm_bindgen_futures::spawn_local(async move { drop(gone) });
        }
    }));
    (slot, stop)
}

/// Run `tick` on every animation frame until it returns `false` or the
/// returned [`Loop`] is stopped.
fn every_frame(tick: impl FnMut() -> bool + 'static) -> Loop {
    let (slot, stop) = loop_slot::<dyn FnMut(f64)>();
    let slot2 = slot.clone();
    let mut tick = tick;
    *slot.borrow_mut() = Some(Closure::new(move |_t: f64| {
        let again = tick();
        let next = again
            .then(|| {
                let w = window()?;
                let b = slot2.borrow();
                w.request_animation_frame(b.as_ref()?.as_ref().unchecked_ref())
                    .ok()
            })
            .flatten();
        if next.is_none() {
            let gone = slot2.borrow_mut().take();
            wasm_bindgen_futures::spawn_local(async move { drop(gone) });
        }
    }));
    let started = window().and_then(|w| {
        let b = slot.borrow();
        w.request_animation_frame(b.as_ref()?.as_ref().unchecked_ref())
            .ok()
    });
    if started.is_none() {
        slot.borrow_mut().take();
    }
    stop
}

// ------------------------------------------------------------ local tracks

/// What keeps an app-fed source running.
enum Source {
    /// A hidden canvas painted from the writer's latest frame by a timer.
    Canvas {
        timer: i32,
        _tick: Closure<dyn FnMut()>,
    },
    /// An AudioContext pulling from the writer's queue.
    Audio {
        ctx: AudioContext,
        node: web_sys::AudioNode,
        _process: Option<JsCallback>,
        timer: Option<i32>,
        _pump: Option<Closure<dyn FnMut()>>,
        /// The context's `statechange` handler (drops the gesture listener
        /// once it runs).
        _on_state: JsCallback,
        /// While the browser holds the context: the window listener that
        /// resumes it on the page's next click, tap or key press.
        gesture: Rc<RefCell<Option<JsCallback>>>,
    },
}

const GESTURES: [&str; 3] = ["pointerdown", "keydown", "touchend"];

fn remove_gesture_listener(cb: &JsCallback) {
    if let Some(w) = window() {
        for ev in GESTURES {
            let _ = w.remove_event_listener_with_callback_and_bool(
                ev,
                cb.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}

/// Let a promise settle with nothing listening, without the browser
/// reporting an unhandled rejection (a `resume()` that `close()` overtook).
fn quiet(p: Result<js_sys::Promise, JsValue>) {
    if let Ok(p) = p {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = JsFuture::from(p).await;
        });
    }
}

struct LocalWeb {
    track: MediaStreamTrack,
    _on_ended: JsCallback,
    source: Option<Source>,
}

thread_local! {
    static LOCAL_WEB: RefCell<HashMap<u64, LocalWeb>> = RefCell::new(HashMap::new());
    static PEERS: RefCell<HashMap<u64, MediaPeer>> = RefCell::new(HashMap::new());
}

fn local_track(key: u64) -> Option<MediaStreamTrack> {
    LOCAL_WEB.with(|l| l.borrow().get(&key).map(|w| Clone::clone(&w.track)))
}

/// Keep `track` for local track `key`, or stop it if the handle is gone.
fn keep_local(key: u64, track: MediaStreamTrack, source: Option<Source>) {
    if !media::local_alive(key) {
        track.stop();
        if let Some(s) = source {
            stop_source(s);
        }
        return;
    }
    let on_ended = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        // The device went away (unplugged, permission revoked).
        media::local_ended(key);
    });
    track.set_onended(Some(on_ended.as_ref().unchecked_ref()));
    let old = LOCAL_WEB.with(|l| {
        l.borrow_mut().insert(
            key,
            LocalWeb {
                track,
                _on_ended: on_ended,
                source,
            },
        )
    });
    if let Some(old) = old {
        release(old);
    }
}

fn stop_source(s: Source) {
    let Some(w) = window() else { return };
    match s {
        Source::Canvas { timer, .. } => w.clear_interval_with_handle(timer),
        Source::Audio {
            ctx,
            node,
            timer,
            gesture,
            ..
        } => {
            if let Some(t) = timer {
                w.clear_interval_with_handle(t);
            }
            if let Some(p) = node.dyn_ref::<ScriptProcessorNode>() {
                p.set_onaudioprocess(None);
            }
            ctx.set_onstatechange(None);
            if let Some(g) = gesture.borrow_mut().take() {
                remove_gesture_listener(&g);
            }
            let _ = node.disconnect();
            quiet(ctx.close());
        }
    }
}

/// Stop a local track's browser side. Its closures are dropped after the
/// current call: this may run inside one of them (an `on_ended` that drops
/// the last handle).
fn release(mut w: LocalWeb) {
    w.track.set_onended(None);
    w.track.stop();
    if let Some(s) = w.source.take() {
        stop_source(s);
    }
    wasm_bindgen_futures::spawn_local(async move { drop(w) });
}

fn gum_error(e: &JsValue) -> MediaError {
    let name = Reflect::get(e, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or_default();
    match name.as_str() {
        "NotAllowedError" | "SecurityError" => MediaError::PermissionDenied,
        "NotFoundError" | "OverconstrainedError" => MediaError::DeviceNotFound(js_err(e)),
        "NotReadableError" | "AbortError" => MediaError::DeviceInUse,
        "TypeError" => MediaError::NotSupported(js_err(e)),
        _ => MediaError::Backend(js_err(e)),
    }
}

async fn get_user_media(constraints: MediaStreamConstraints) -> Result<MediaStream, MediaError> {
    let devices = window()
        .ok_or_else(|| MediaError::NotSupported("no window".into()))?
        .navigator()
        .media_devices()
        .map_err(|_| {
            MediaError::NotSupported("navigator.mediaDevices (needs a secure context)".into())
        })?;
    let promise = devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|e| gum_error(&e))?;
    JsFuture::from(promise)
        .await
        .map(|s| s.unchecked_into())
        .map_err(|e| gum_error(&e))
}

fn first_track(stream: &MediaStream, kind: MediaKind) -> Option<MediaStreamTrack> {
    let tracks = match kind {
        MediaKind::Audio => stream.get_audio_tracks(),
        MediaKind::Video => stream.get_video_tracks(),
    };
    tracks.get(0).dyn_into().ok()
}

/// Ask for `constraints` and keep the first `kind` track for local track
/// `key`. The request runs in its own task, not in the returned future: a
/// future dropped while the permission prompt is open (its component
/// unmounted) must not leave the device the browser grants later running.
/// `keep_local` stops a track whose `LocalTrack` is gone.
fn request(
    key: u64,
    constraints: MediaStreamConstraints,
    kind: MediaKind,
) -> impl Future<Output = Result<(), MediaError>> + 'static {
    let (tx, rx) = oneshot();
    wasm_bindgen_futures::spawn_local(async move {
        let result = async {
            let stream = get_user_media(constraints).await?;
            let track = first_track(&stream, kind)
                .ok_or_else(|| MediaError::DeviceNotFound(format!("no {kind:?} track")))?;
            // Any other tracks the stream carries are not ours to keep.
            for t in stream.get_tracks().iter() {
                if let Ok(t) = t.dyn_into::<MediaStreamTrack>()
                    && !Object::is(&t, &track)
                {
                    t.stop();
                }
            }
            keep_local(key, track, None);
            Ok(())
        }
        .await;
        tx.send(result);
    });
    async move {
        rx.await
            .unwrap_or_else(|| Err(MediaError::Backend("the request was dropped".into())))
    }
}

pub(crate) fn open_camera(
    key: u64,
    o: CameraOptions,
) -> impl Future<Output = Result<(), MediaError>> + 'static {
    let video = MediaTrackConstraints::new();
    video.set_width_i32(o.width as i32);
    video.set_height_i32(o.height as i32);
    video.set_frame_rate_f64(o.fps as f64);
    if let Some(d) = &o.device {
        let exact = Object::new();
        let _ = Reflect::set(&exact, &"exact".into(), &d.into());
        video.set_device_id(&exact);
    }
    let c = MediaStreamConstraints::new();
    c.set_video(&video);
    request(key, c, MediaKind::Video)
}

pub(crate) fn open_microphone(
    key: u64,
    o: MicrophoneOptions,
) -> impl Future<Output = Result<(), MediaError>> + 'static {
    let c = MediaStreamConstraints::new();
    match &o.device {
        Some(d) => {
            let audio = MediaTrackConstraints::new();
            let exact = Object::new();
            let _ = Reflect::set(&exact, &"exact".into(), &d.into());
            audio.set_device_id(&exact);
            c.set_audio(&audio);
        }
        None => c.set_audio(&JsValue::TRUE),
    }
    request(key, c, MediaKind::Audio)
}

/// A one-shot channel on the one web thread.
struct OneShot<T> {
    value: Option<T>,
    closed: bool,
    waker: Option<std::task::Waker>,
}

struct OneTx<T>(Rc<RefCell<OneShot<T>>>);
struct OneRx<T>(Rc<RefCell<OneShot<T>>>);

fn oneshot<T>() -> (OneTx<T>, OneRx<T>) {
    let s = Rc::new(RefCell::new(OneShot {
        value: None,
        closed: false,
        waker: None,
    }));
    (OneTx(s.clone()), OneRx(s))
}

impl<T> OneTx<T> {
    fn send(self, v: T) {
        let w = {
            let mut s = self.0.borrow_mut();
            s.value = Some(v);
            s.waker.take()
        };
        if let Some(w) = w {
            w.wake();
        }
    }
}

impl<T> Drop for OneTx<T> {
    fn drop(&mut self) {
        let w = {
            let mut s = self.0.borrow_mut();
            s.closed = true;
            s.waker.take()
        };
        if let Some(w) = w {
            w.wake();
        }
    }
}

impl<T> Future for OneRx<T> {
    type Output = Option<T>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<T>> {
        let mut s = self.0.borrow_mut();
        if let Some(v) = s.value.take() {
            return std::task::Poll::Ready(Some(v));
        }
        if s.closed {
            return std::task::Poll::Ready(None);
        }
        s.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

pub(crate) fn open_video_source(
    key: u64,
    o: VideoSourceOptions,
    w: VideoSourceWriter,
) -> Result<(), MediaError> {
    let backend = |e: JsValue| MediaError::Backend(js_err(&e));
    let doc = document().ok_or_else(|| MediaError::NotSupported("no document".into()))?;
    let canvas: HtmlCanvasElement = doc
        .create_element("canvas")
        .map_err(backend)?
        .unchecked_into();
    canvas.set_width(o.width.max(1));
    canvas.set_height(o.height.max(1));
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(backend)?
        .ok_or_else(|| MediaError::NotSupported("canvas 2d".into()))?
        .unchecked_into();
    // Black until the first frame.
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, o.width as f64, o.height as f64);
    let fps = o.fps.clamp(1, 120);
    let stream = canvas
        .capture_stream_with_frame_request_rate(fps as f64)
        .map_err(|e| MediaError::NotSupported(format!("canvas.captureStream: {}", js_err(&e))))?;
    let track = first_track(&stream, MediaKind::Video)
        .ok_or_else(|| MediaError::Backend("captureStream gave no track".into()))?;

    // putImageData has to run here, on the UI thread: a timer copies the
    // writer's latest frame in when it is a new one.
    let last = Cell::new(0u64);
    let slot = w.slot.clone();
    let tick = Closure::<dyn FnMut()>::new(move || {
        let seq = slot.seq.load(Ordering::Acquire);
        if seq == last.get() {
            return;
        }
        last.set(seq);
        let Ok(guard) = slot.latest.lock() else {
            return;
        };
        let Some((rgba, fw, fh)) = guard.as_ref() else {
            return;
        };
        if canvas.width() != *fw || canvas.height() != *fh {
            canvas.set_width(*fw);
            canvas.set_height(*fh);
        }
        let img = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
            wasm_bindgen::Clamped(rgba.as_slice()),
            *fw,
            *fh,
        );
        if let Ok(img) = img
            && ctx.put_image_data(&img, 0.0, 0.0).is_ok()
        {
            slot.taken.fetch_add(1, Ordering::Relaxed);
        }
    });
    let timer = window()
        .ok_or_else(|| MediaError::NotSupported("no window".into()))?
        .set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            (1000 / fps) as i32,
        )
        .map_err(backend)?;
    keep_local(key, track, Some(Source::Canvas { timer, _tick: tick }));
    Ok(())
}

/// The AudioWorklet processor an app-fed audio source plays through. It
/// keeps at most one second queued.
const WORKLET_SOURCE: &str = r#"
class RinchRtcSource extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.channels = options.processorOptions.channels;
    this.cap = options.processorOptions.cap;
    this.chunks = [];
    this.offset = 0;
    this.queued = 0;
    this.port.onmessage = (e) => {
      this.chunks.push(e.data);
      this.queued += e.data.length;
      while (this.queued - this.offset > this.cap && this.chunks.length > 1) {
        this.queued -= this.chunks[0].length;
        this.chunks.shift();
        this.offset = 0;
      }
    };
  }
  process(inputs, outputs) {
    const out = outputs[0];
    const n = out[0].length;
    for (let i = 0; i < n; i++) {
      for (let c = 0; c < out.length; c++) {
        let v = 0;
        if (c < this.channels && this.chunks.length > 0) {
          v = this.chunks[0][this.offset];
          this.offset++;
          if (this.offset >= this.chunks[0].length) {
            this.queued -= this.chunks[0].length;
            this.chunks.shift();
            this.offset = 0;
          }
        }
        out[c][i] = v;
      }
      // A source with more channels than the output: skip the extras.
      for (let c = out.length; c < this.channels && this.chunks.length > 0; c++) {
        this.offset++;
        if (this.offset >= this.chunks[0].length) {
          this.queued -= this.chunks[0].length;
          this.chunks.shift();
          this.offset = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor("rinch-rtc-source", RinchRtcSource);
"#;

/// Take up to `max` samples off the queue.
fn drain(queue: &AudioQueue, max: usize) -> Vec<f32> {
    let Ok(mut q) = queue.samples.lock() else {
        return Vec::new();
    };
    let n = q.len().min(max);
    queue.taken.fetch_add(n as u64, Ordering::Relaxed);
    q.drain(..n).collect()
}

pub(crate) fn open_audio_source(
    key: u64,
    o: AudioSourceOptions,
    w: AudioSourceWriter,
) -> Result<(), MediaError> {
    let backend = |e: JsValue| MediaError::Backend(js_err(&e));
    let opts = AudioContextOptions::new();
    opts.set_sample_rate(o.sample_rate as f32);
    let ctx = AudioContext::new_with_context_options(&opts)
        .map_err(|e| MediaError::NotSupported(format!("AudioContext: {}", js_err(&e))))?;
    let dest: MediaStreamAudioDestinationNode =
        ctx.create_media_stream_destination().map_err(backend)?;
    let track = first_track(&dest.stream(), MediaKind::Audio)
        .ok_or_else(|| MediaError::Backend("no audio track from the destination".into()))?;
    let channels = u32::from(w.queue.channels);
    // A context made outside a user gesture starts suspended and sends
    // silence, with no error, while the track reads `live`. Ask it to run;
    // if the browser holds it, `LocalTrack::status` says
    // `WaitingForGesture` and the page's next click, tap or key press
    // resumes it.
    quiet(ctx.resume());
    let gesture: Rc<RefCell<Option<JsCallback>>> = Rc::default();
    if ctx.state() != web_sys::AudioContextState::Running {
        let c = ctx.clone();
        let cb = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| quiet(c.resume()));
        if let Some(win) = window() {
            for ev in GESTURES {
                let _ = win.add_event_listener_with_callback_and_bool(
                    ev,
                    cb.as_ref().unchecked_ref(),
                    true,
                );
            }
        }
        *gesture.borrow_mut() = Some(cb);
    }
    let (c, g) = (ctx.clone(), Rc::downgrade(&gesture));
    let on_state = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        if c.state() == web_sys::AudioContextState::Running
            && let Some(g) = g.upgrade()
            && let Some(cb) = g.borrow_mut().take()
        {
            remove_gesture_listener(&cb);
            // Dropped after this call: it may be on the stack.
            wasm_bindgen_futures::spawn_local(async move { drop(cb) });
        }
    });
    ctx.set_onstatechange(Some(on_state.as_ref().unchecked_ref()));

    // The ScriptProcessorNode path: synchronous, so the track exists with
    // its source attached. (An AudioWorklet module loads asynchronously;
    // see `upgrade_to_worklet`.)
    let proc = ctx
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
            1024, 0, channels,
        )
        .map_err(backend)?;
    let queue = w.queue.clone();
    let process = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        let Ok(e) = e.dyn_into::<AudioProcessingEvent>() else {
            return;
        };
        let Ok(out) = e.output_buffer() else { return };
        let frames = out.length() as usize;
        let ch = out.number_of_channels() as usize;
        let samples = drain(&queue, frames * ch);
        for c in 0..ch {
            let mut plane = vec![0.0f32; frames];
            for (i, v) in plane.iter_mut().enumerate() {
                if let Some(s) = samples.get(i * ch + c) {
                    *v = *s;
                }
            }
            let _ = out.copy_to_channel(&plane, c as i32);
        }
    });
    proc.set_onaudioprocess(Some(process.as_ref().unchecked_ref()));
    proc.connect_with_audio_node(&dest).map_err(backend)?;
    keep_local(
        key,
        track,
        Some(Source::Audio {
            ctx: ctx.clone(),
            node: proc.clone().unchecked_into(),
            _process: Some(process),
            timer: None,
            _pump: None,
            _on_state: on_state,
            gesture,
        }),
    );
    wasm_bindgen_futures::spawn_local(upgrade_to_worklet(key, ctx, dest, channels, w));
    Ok(())
}

/// Swap the ScriptProcessorNode for an AudioWorkletNode once the module has
/// loaded (where the browser has AudioWorklet); keep the processor if not.
async fn upgrade_to_worklet(
    key: u64,
    ctx: AudioContext,
    dest: MediaStreamAudioDestinationNode,
    channels: u32,
    w: AudioSourceWriter,
) {
    let Ok(worklet) = ctx.audio_worklet() else {
        return;
    };
    let parts: Array = [JsValue::from_str(WORKLET_SOURCE)].into_iter().collect();
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("application/javascript");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &bag) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    let loaded = match worklet.add_module(&url) {
        Ok(p) => JsFuture::from(p).await,
        Err(e) => Err(e),
    };
    let _ = web_sys::Url::revoke_object_url(&url);
    if let Err(e) = loaded {
        tracing::debug!(
            "rinch-rtc: AudioWorklet unavailable, keeping ScriptProcessor: {}",
            js_err(&e)
        );
        return;
    }
    // Still the same source? (stopped or dropped meanwhile: nothing to do)
    let current = LOCAL_WEB.with(|l| {
        l.borrow()
            .get(&key)
            .and_then(|lw| match &lw.source {
                Some(Source::Audio { ctx: c, .. }) => Some(Object::is(c, &ctx)),
                _ => None,
            })
            .unwrap_or(false)
    });
    if !current {
        return;
    }
    let opts = web_sys::AudioWorkletNodeOptions::new();
    opts.set_number_of_inputs(0);
    opts.set_number_of_outputs(1);
    let out_ch: Array = [JsValue::from(channels)].into_iter().collect();
    opts.set_output_channel_count(&out_ch);
    let po = Object::new();
    let cap = w.queue.sample_rate as usize * channels as usize;
    let _ = Reflect::set(&po, &"channels".into(), &channels.into());
    let _ = Reflect::set(&po, &"cap".into(), &(cap as u32).into());
    opts.set_processor_options(Some(&po));
    let Ok(node) = web_sys::AudioWorkletNode::new_with_options(&ctx, "rinch-rtc-source", &opts)
    else {
        return;
    };
    let Ok(port) = node.port() else { return };
    if node.connect_with_audio_node(&dest).is_err() {
        return;
    }
    // The pump: every 10 ms, hand what the writer queued to the worklet.
    let queue = w.queue.clone();
    let pump = Closure::<dyn FnMut()>::new(move || {
        let samples = drain(&queue, usize::MAX);
        if !samples.is_empty() {
            let arr = js_sys::Float32Array::from(samples.as_slice());
            let _ = port.post_message(&arr);
        }
    });
    let Some(timer) = window().and_then(|w| {
        w.set_interval_with_callback_and_timeout_and_arguments_0(pump.as_ref().unchecked_ref(), 10)
            .ok()
    }) else {
        let _ = node.disconnect();
        return;
    };
    LOCAL_WEB.with(|l| {
        if let Some(lw) = l.borrow_mut().get_mut(&key)
            && let Some(Source::Audio {
                node: n,
                _process,
                timer: t,
                _pump,
                ..
            }) = &mut lw.source
        {
            if let Some(p) = n.dyn_ref::<ScriptProcessorNode>() {
                p.set_onaudioprocess(None);
            }
            let _ = n.disconnect();
            *n = node.unchecked_into();
            *_process = None;
            *t = Some(timer);
            *_pump = Some(pump);
        }
    });
}

/// Whether the browser holds this local track's source until a gesture
/// (an app-fed audio source whose AudioContext is suspended).
pub(crate) fn waiting_for_gesture(key: u64) -> bool {
    LOCAL_WEB.with(|l| {
        l.borrow().get(&key).is_some_and(|w| {
            matches!(&w.source, Some(Source::Audio { ctx, .. })
                if ctx.state() == web_sys::AudioContextState::Suspended)
        })
    })
}

pub(crate) fn set_local_enabled(key: u64, enabled: bool) {
    if let Some(t) = local_track(key) {
        t.set_enabled(enabled);
    }
}

pub(crate) fn stop_local(key: u64) {
    // `try_with`: a track dropped during thread exit has nothing left to stop.
    let gone = LOCAL_WEB
        .try_with(|l| l.borrow_mut().remove(&key))
        .ok()
        .flatten();
    if let Some(w) = gone {
        release(w);
    }
}

// ------------------------------------------------------------ per peer

struct Tx {
    key: u64,
    sender: u64,
    tx: RtcRtpTransceiver,
}

struct RemoteWeb {
    track: MediaStreamTrack,
    stream: MediaStream,
    audio: Option<HtmlAudioElement>,
    /// An `on_frame` read-back: its loop and its hidden `<video>`.
    frames: Option<FrameReader>,
    _closures: Vec<JsCallback>,
}

impl RemoteWeb {
    fn detach(&self) {
        self.track.set_onmute(None);
        self.track.set_onunmute(None);
        self.track.set_onended(None);
        if let Some(a) = &self.audio {
            let _ = a.pause();
            a.set_src_object(None);
            a.remove();
        }
        if let Some(f) = &self.frames {
            f.stop();
        }
    }
}

struct FrameReader {
    frames: Loop,
    video: HtmlVideoElement,
}

impl FrameReader {
    /// Break the loop (its closure holds itself, the canvas and the app's
    /// callback) and let the hidden `<video>` go.
    fn stop(&self) {
        self.frames.stop();
        let _ = self.video.pause();
        self.video.set_src_object(None);
    }
}

struct MediaPeer {
    pc: RtcPeerConnection,
    /// One stream for everything this side sends, so the far side sees one
    /// person's audio and video grouped (`RemoteTrack::stream_ids`).
    stream: MediaStream,
    txs: Vec<Tx>,
    senders: HashMap<u64, RtcRtpSender>,
    remotes: HashMap<u64, RemoteWeb>,
    _closures: Vec<JsCallback>,
}

fn kind_of(tx: &RtcRtpTransceiver) -> MediaKind {
    if tx.receiver().track().kind() == "audio" {
        MediaKind::Audio
    } else {
        MediaKind::Video
    }
}

/// The keys of `tx` on peer `p`, registering it when it is new (one the far
/// side's offer created).
fn tx_keys(p: &mut MediaPeer, tx: &RtcRtpTransceiver) -> (u64, u64) {
    if let Some(t) = p.txs.iter().find(|t| Object::is(&t.tx, tx)) {
        return (t.key, t.sender);
    }
    let (key, sender) = (crate::next_id(), crate::next_id());
    p.senders.insert(sender, tx.sender());
    p.txs.push(Tx {
        key,
        sender,
        tx: tx.clone(),
    });
    (key, sender)
}

fn with_peer<R>(id: u64, f: impl FnOnce(&mut MediaPeer) -> R) -> Option<R> {
    PEERS.with(|p| p.borrow_mut().get_mut(&id).map(f))
}

fn to_js_direction(d: Direction) -> RtcRtpTransceiverDirection {
    match d {
        Direction::SendRecv => RtcRtpTransceiverDirection::Sendrecv,
        Direction::SendOnly => RtcRtpTransceiverDirection::Sendonly,
        Direction::RecvOnly => RtcRtpTransceiverDirection::Recvonly,
        Direction::Inactive => RtcRtpTransceiverDirection::Inactive,
    }
}

fn from_js_direction(d: RtcRtpTransceiverDirection) -> Option<Direction> {
    match d {
        RtcRtpTransceiverDirection::Sendrecv => Some(Direction::SendRecv),
        RtcRtpTransceiverDirection::Sendonly => Some(Direction::SendOnly),
        RtcRtpTransceiverDirection::Recvonly => Some(Direction::RecvOnly),
        RtcRtpTransceiverDirection::Inactive => Some(Direction::Inactive),
        // "stopped"
        _ => None,
    }
}

/// Called by `wasm.rs` when peer `id` is created: hook `ontrack` and
/// `negotiationneeded`.
pub(crate) fn attach(id: u64, pc: &RtcPeerConnection) {
    let on_track = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        if let Ok(e) = e.dyn_into::<RtcTrackEvent>() {
            remote_track(id, e);
        }
    });
    pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
    let on_negotiation = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        registry::dispatch(id, Event::Media(MediaEvent::NegotiationNeeded));
    });
    pc.set_onnegotiationneeded(Some(on_negotiation.as_ref().unchecked_ref()));
    let stream = MediaStream::new().unwrap_or_else(|_| Object::new().unchecked_into());
    PEERS.with(|p| {
        p.borrow_mut().insert(
            id,
            MediaPeer {
                pc: pc.clone(),
                stream,
                txs: Vec::new(),
                senders: HashMap::new(),
                remotes: HashMap::new(),
                _closures: vec![on_track, on_negotiation],
            },
        )
    });
}

/// Called by `wasm.rs` as peer `id` closes: detach everything, remove the
/// remote audio elements.
pub(crate) fn detach(id: u64) {
    let peer = PEERS.with(|p| p.borrow_mut().remove(&id));
    let Some(peer) = peer else { return };
    peer.pc.set_ontrack(None);
    peer.pc.set_onnegotiationneeded(None);
    for r in peer.remotes.values() {
        r.detach();
    }
    // Dropped after the current call, which may be one of its closures.
    wasm_bindgen_futures::spawn_local(async move { drop(peer) });
}

fn remote_track(id: u64, e: RtcTrackEvent) {
    let track = e.track();
    let streams = e.streams();
    let stream_ids: Vec<String> = streams
        .iter()
        .filter_map(|s| s.dyn_into::<MediaStream>().ok())
        .map(|s| s.id())
        .collect();
    let kind = if track.kind() == "audio" {
        MediaKind::Audio
    } else {
        MediaKind::Video
    };
    // The same browser track again: a transceiver that went inactive and
    // came back fires `track` once more. It is one remote track, already
    // handed over (its unmute reports the return), with one `<audio>`.
    let known = with_peer(id, |p| {
        p.remotes.values().any(|r| Object::is(&r.track, &track))
    });
    if known != Some(false) {
        return;
    }
    let key = crate::next_id();
    let Some(tx_key) = with_peer(id, |p| tx_keys(p, &e.transceiver()).0) else {
        return;
    };
    // A stream with only this track, for the elements that play it.
    let own: Array = [JsValue::from(Clone::clone(&track))].into_iter().collect();
    let Ok(stream) = MediaStream::new_with_tracks(&own) else {
        return;
    };

    let mk = |ev: fn(u64) -> MediaEvent| {
        Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
            registry::dispatch(id, Event::Media(ev(key)));
        })
    };
    let on_mute = mk(|key| MediaEvent::Muted { key, muted: true });
    let on_unmute = mk(|key| MediaEvent::Muted { key, muted: false });
    let on_ended = Closure::<dyn FnMut(JsValue)>::new(move |_e: JsValue| {
        let gone = with_peer(id, |p| {
            p.remotes
                .get_mut(&key)
                .map(|r| (r.audio.take(), r.frames.take()))
        });
        if let Some(Some((audio, frames))) = gone {
            if let Some(a) = audio {
                a.set_src_object(None);
                a.remove();
            }
            if let Some(f) = frames {
                f.stop();
            }
        }
        registry::dispatch(id, Event::Media(MediaEvent::Ended { key }));
    });
    track.set_onmute(Some(on_mute.as_ref().unchecked_ref()));
    track.set_onunmute(Some(on_unmute.as_ref().unchecked_ref()));
    track.set_onended(Some(on_ended.as_ref().unchecked_ref()));

    let audio = (kind == MediaKind::Audio)
        .then(|| play_audio(id, &stream))
        .flatten();
    let muted = track.muted();
    let track_id = track.id();
    with_peer(id, |p| {
        p.remotes.insert(
            key,
            RemoteWeb {
                track,
                stream,
                audio,
                frames: None,
                _closures: vec![on_mute, on_unmute, on_ended],
            },
        )
    });
    registry::dispatch(
        id,
        Event::Media(MediaEvent::Track {
            key,
            kind,
            id: track_id,
            stream_ids,
            transceiver: tx_key,
            muted,
        }),
    );
}

/// A hidden `<audio autoplay>` in the body, playing `stream`. A refused
/// `play()` (the autoplay policy) is reported through `Peer::on_error`.
fn play_audio(id: u64, stream: &MediaStream) -> Option<HtmlAudioElement> {
    let doc = document()?;
    let audio: HtmlAudioElement = doc.create_element("audio").ok()?.unchecked_into();
    audio.set_autoplay(true);
    let _ = audio.set_attribute("data-rinch-rtc-audio", "");
    let _ = audio.set_attribute("style", "display: none");
    audio.set_src_object(Some(stream));
    doc.body()?.append_child(&audio).ok()?;
    start_playing(id, &audio);
    Some(audio)
}

fn start_playing(id: u64, audio: &HtmlAudioElement) {
    if let Ok(p) = audio.play() {
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = JsFuture::from(p).await {
                registry::dispatch(
                    id,
                    Event::Error(RtcError::Media(format!(
                        "remote audio did not play (autoplay policy?): {}; \
                         call RemoteTrack::set_playing(true) from a click",
                        js_err(&e)
                    ))),
                );
            }
        });
    }
}

pub(crate) fn add_track(p: u64, l: u64, s: u64, t: u64) -> Result<(), RtcError> {
    let track =
        local_track(l).ok_or_else(|| RtcError::InvalidState("the track is not open".into()))?;
    with_peer(p, |peer| {
        let sender = call(&peer.pc, "addTrack", &[&track, &peer.stream])
            .map_err(|e| RtcError::InvalidState(js_err(&e)))?;
        let sender: RtcRtpSender = sender.unchecked_into();
        peer.senders.insert(s, sender.clone());
        let tx = peer
            .pc
            .get_transceivers()
            .iter()
            .filter_map(|t| t.dyn_into::<RtcRtpTransceiver>().ok())
            .find(|t| Object::is(&t.sender(), &sender));
        if let Some(tx) = tx
            && !peer.txs.iter().any(|x| Object::is(&x.tx, &tx))
        {
            peer.txs.push(Tx {
                key: t,
                sender: s,
                tx,
            });
        }
        Ok(())
    })
    .unwrap_or(Err(RtcError::Closed))
}

pub(crate) fn add_transceiver(
    p: u64,
    k: MediaKind,
    d: Direction,
    s: u64,
    t: u64,
) -> Result<(), RtcError> {
    with_peer(p, |peer| {
        let init = RtcRtpTransceiverInit::new();
        init.set_direction(to_js_direction(d));
        let streams: Array = [JsValue::from(Clone::clone(&peer.stream))]
            .into_iter()
            .collect();
        init.set_streams(&streams);
        let kind = JsValue::from_str(match k {
            MediaKind::Audio => "audio",
            MediaKind::Video => "video",
        });
        let tx = call(&peer.pc, "addTransceiver", &[&kind, &init])
            .map_err(|e| RtcError::InvalidState(js_err(&e)))?;
        let tx: RtcRtpTransceiver = tx.unchecked_into();
        peer.senders.insert(s, tx.sender());
        peer.txs.push(Tx {
            key: t,
            sender: s,
            tx,
        });
        Ok(())
    })
    .unwrap_or(Err(RtcError::Closed))
}

pub(crate) fn transceivers(p: u64) -> Vec<(u64, u64, MediaKind)> {
    with_peer(p, |peer| {
        let all: Vec<RtcRtpTransceiver> = peer
            .pc
            .get_transceivers()
            .iter()
            .filter_map(|t| t.dyn_into().ok())
            .collect();
        all.iter()
            .map(|tx| {
                let (key, sender) = tx_keys(peer, tx);
                (key, sender, kind_of(tx))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn with_tx<R>(p: u64, t: u64, f: impl FnOnce(&RtcRtpTransceiver) -> R) -> Option<R> {
    with_peer(p, |peer| {
        peer.txs.iter().find(|x| x.key == t).map(|x| f(&x.tx))
    })
    .flatten()
}

fn sender(p: u64, s: u64) -> Option<RtcRtpSender> {
    with_peer(p, |peer| peer.senders.get(&s).cloned()).flatten()
}

pub(crate) fn replace_track(
    p: u64,
    s: u64,
    l: Option<u64>,
) -> impl Future<Output = Result<(), RtcError>> + 'static {
    let started = (|| {
        let sender = sender(p, s).ok_or(RtcError::Closed)?;
        let track = match l {
            Some(l) => Some(
                local_track(l)
                    .ok_or_else(|| RtcError::InvalidState("the track is not open".into()))?,
            ),
            None => None,
        };
        Ok(sender.replace_track(track.as_ref()))
    })();
    async move {
        let promise = started?;
        JsFuture::from(promise)
            .await
            .map(|_| ())
            .map_err(|e| RtcError::InvalidState(js_err(&e)))
    }
}

pub(crate) fn set_max_bitrate(p: u64, s: u64, bps: Option<u32>) {
    let Some(sender) = sender(p, s) else { return };
    let params: JsValue = sender.get_parameters().into();
    let encodings = Reflect::get(&params, &"encodings".into())
        .ok()
        .and_then(|e| e.dyn_into::<Array>().ok());
    let Some(encodings) = encodings.filter(|e| e.length() > 0) else {
        // Before negotiation an engine may list no encodings, and adding
        // one is refused; the cap applies from the next call.
        tracing::debug!("rinch-rtc: set_max_bitrate before the sender has encodings; ignored");
        return;
    };
    for enc in encodings.iter() {
        match bps {
            Some(b) => {
                let _ = Reflect::set(&enc, &"maxBitrate".into(), &b.into());
            }
            None => {
                let _ =
                    Reflect::delete_property(enc.unchecked_ref::<Object>(), &"maxBitrate".into());
            }
        }
    }
    match call(&sender, "setParameters", &[&params]) {
        Ok(promise) => wasm_bindgen_futures::spawn_local(async move {
            if let Ok(p) = promise.dyn_into::<js_sys::Promise>()
                && let Err(e) = JsFuture::from(p).await
            {
                tracing::warn!("rinch-rtc: setParameters refused: {}", js_err(&e));
            }
        }),
        Err(e) => tracing::warn!("rinch-rtc: setParameters refused: {}", js_err(&e)),
    }
}

pub(crate) fn transceiver_mid(p: u64, t: u64) -> Option<String> {
    with_tx(p, t, |tx| tx.mid()).flatten()
}

pub(crate) fn transceiver_direction(p: u64, t: u64) -> Direction {
    with_tx(p, t, |tx| from_js_direction(tx.direction()))
        .flatten()
        .unwrap_or(Direction::Inactive)
}

pub(crate) fn set_transceiver_direction(p: u64, t: u64, d: Direction) {
    with_tx(p, t, |tx| {
        if !tx.stopped() {
            tx.set_direction(to_js_direction(d))
        }
    });
}

pub(crate) fn transceiver_current_direction(p: u64, t: u64) -> Option<Direction> {
    with_tx(p, t, |tx| {
        tx.current_direction().and_then(from_js_direction)
    })
    .flatten()
}

pub(crate) fn stop_transceiver(p: u64, t: u64) {
    with_tx(p, t, |tx| {
        let _ = call(tx, "stop", &[]);
    });
}

pub(crate) fn transceiver_sender(p: u64, t: u64) -> Option<u64> {
    with_peer(p, |peer| {
        peer.txs.iter().find(|x| x.key == t).map(|x| x.sender)
    })
    .flatten()
}

fn remote_audio(p: u64, k: u64) -> Option<HtmlAudioElement> {
    with_peer(p, |peer| peer.remotes.get(&k).and_then(|r| r.audio.clone())).flatten()
}

pub(crate) fn set_remote_volume(p: u64, k: u64, v: f32) {
    if let Some(a) = remote_audio(p, k) {
        a.set_volume(v as f64);
    }
}

pub(crate) fn set_remote_playing(p: u64, k: u64, playing: bool) {
    if let Some(a) = remote_audio(p, k) {
        if playing {
            start_playing(p, &a);
        } else {
            let _ = a.pause();
        }
    }
}

/// Read a remote video track back: a hidden `<video>` drawn into a canvas
/// on each animation frame that has a new video frame. Costly; for tests
/// and analysis.
pub(crate) fn on_remote_frame(p: u64, k: u64, cb: media::FrameCallback) {
    let Some(stream) = with_peer(p, |peer| {
        let r = peer.remotes.get_mut(&k)?;
        if let Some(old) = r.frames.take() {
            old.stop();
        }
        Some(Clone::clone(&r.stream))
    })
    .flatten() else {
        return;
    };
    let Some(doc) = document() else { return };
    let Ok(video) = doc.create_element("video") else {
        return;
    };
    let video: HtmlVideoElement = video.unchecked_into();
    video.set_muted(true);
    video.set_autoplay(true);
    let _ = video.set_attribute("playsinline", "");
    video.set_src_object(Some(&stream));
    let _ = video.play();
    let Ok(canvas) = doc.create_element("canvas") else {
        return;
    };
    let canvas: HtmlCanvasElement = canvas.unchecked_into();
    let ctx_opts = Object::new();
    let _ = Reflect::set(&ctx_opts, &"willReadFrequently".into(), &JsValue::TRUE);
    let Some(ctx) = canvas
        .get_context_with_context_options("2d", &ctx_opts)
        .ok()
        .flatten()
        .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
    else {
        return;
    };
    let mut cb = cb;
    let v2 = video.clone();
    let frames = every_video_frame(video.clone(), move || {
        let (w, h) = (v2.video_width(), v2.video_height());
        if w == 0 || h == 0 {
            return true;
        }
        if canvas.width() != w || canvas.height() != h {
            canvas.set_width(w);
            canvas.set_height(h);
        }
        if ctx
            .draw_image_with_html_video_element(&v2, 0.0, 0.0)
            .is_err()
        {
            return true;
        }
        if let Ok(img) = ctx.get_image_data(0.0, 0.0, w as f64, h as f64) {
            cb(&img.data(), w, h);
        }
        true
    });
    let mut reader = Some(FrameReader { frames, video });
    with_peer(p, |peer| {
        if let Some(r) = peer.remotes.get_mut(&k) {
            r.frames = reader.take();
        }
    });
    // The track or the peer went meanwhile: nothing would stop it later.
    if let Some(r) = reader {
        r.stop();
    }
}

/// Run `tick` once per new video frame `video` presents, until it returns
/// `false`: `requestVideoFrameCallback` where the browser has it (a live
/// stream's `currentTime` advances with the clock, frames or not, so it
/// cannot tell a new frame), else every animation frame.
fn every_video_frame(video: HtmlVideoElement, tick: impl FnMut() -> bool + 'static) -> Loop {
    let has_rvfc = Reflect::get(&video, &"requestVideoFrameCallback".into())
        .map(|f| f.is_function())
        .unwrap_or(false);
    if !has_rvfc {
        return every_frame(tick);
    }
    let (slot, stop) = loop_slot::<dyn FnMut(JsValue, JsValue)>();
    let (slot2, v2) = (slot.clone(), video.clone());
    let mut tick = tick;
    *slot.borrow_mut() = Some(Closure::new(move |_now: JsValue, _meta: JsValue| {
        let again = tick() && {
            let b = slot2.borrow();
            b.as_ref()
                .map(|c| call(&v2, "requestVideoFrameCallback", &[c.as_ref()]).is_ok())
                .unwrap_or(false)
        };
        if !again {
            let gone = slot2.borrow_mut().take();
            wasm_bindgen_futures::spawn_local(async move { drop(gone) });
        }
    }));
    let started = {
        let b = slot.borrow();
        b.as_ref()
            .map(|c| call(&video, "requestVideoFrameCallback", &[c.as_ref()]).is_ok())
            .unwrap_or(false)
    };
    if !started {
        slot.borrow_mut().take();
    }
    stop
}

// ------------------------------------------------------------- the view

fn view_track(t: &MediaTrack) -> Option<MediaStreamTrack> {
    match t {
        MediaTrack::None => None,
        MediaTrack::Local(l) => local_track(l.key()),
        MediaTrack::Remote(r) => with_peer(r.peer, |p| {
            p.remotes.get(&r.key).map(|x| Clone::clone(&x.track))
        })
        .flatten(),
    }
}

/// A `<video autoplay playsinline muted>`; once it is in the page, found by
/// its `data-rtc-view` attribute (rinch-web has no `NodeHandle`-to-element
/// API), its `srcObject` is set to a stream holding the track.
pub(crate) fn create_view(scope: &mut RenderScope, view_id: &str, t: &MediaTrack) -> NodeHandle {
    let el = scope.create_element("video");
    el.set_attribute("data-rtc-view", view_id);
    el.set_attribute("autoplay", "");
    el.set_attribute("playsinline", "");
    el.set_attribute("muted", "");
    let Some(track) = view_track(t) else {
        return el;
    };
    let own: Array = [JsValue::from(track)].into_iter().collect();
    let Ok(stream) = MediaStream::new_with_tracks(&own) else {
        return el;
    };
    let selector = format!("[data-rtc-view=\"{view_id}\"]");
    // The element once found, so unmounting can let go of the track.
    let shown: Rc<RefCell<Option<HtmlVideoElement>>> = Rc::default();
    let shown2 = shown.clone();
    // Mounting is not synchronous on every path: look for it each frame,
    // for up to ~5 s.
    let mut frames = 0u32;
    let lookup = every_frame(move || {
        frames += 1;
        let found = document()
            .and_then(|d| d.query_selector(&selector).ok().flatten())
            .and_then(|e| e.dyn_into::<HtmlVideoElement>().ok());
        match found {
            Some(video) => {
                // Muted: an unmuted <video> may be refused autoplay, and
                // remote audio has its own element. The `muted` attribute
                // already does it in Chrome and Firefox here (rinch-web sets
                // it before the element is connected; measured: removing
                // this line changes no test); the property is set too, for
                // a DOM that does not carry the attribute through.
                video.set_muted(true);
                video.set_src_object(Some(&stream));
                let _ = video.play();
                *shown2.borrow_mut() = Some(video);
                false
            }
            None => frames < 300,
        }
    });
    // Unmounted: stop looking, and clear `srcObject` so the element (which
    // the page may keep a while) does not hold the track.
    scope.on_cleanup(move || {
        lookup.stop();
        if let Some(v) = shown.borrow_mut().take() {
            let _ = v.pause();
            v.set_src_object(None);
        }
    });
    el
}

pub(crate) async fn devices(kind: MediaKind) -> Vec<DeviceInfo> {
    let Some(devices) = window().and_then(|w| w.navigator().media_devices().ok()) else {
        return Vec::new();
    };
    let Ok(promise) = devices.enumerate_devices() else {
        return Vec::new();
    };
    let Ok(list) = JsFuture::from(promise).await else {
        return Vec::new();
    };
    let want = match kind {
        MediaKind::Audio => MediaDeviceKind::Audioinput,
        MediaKind::Video => MediaDeviceKind::Videoinput,
    };
    Array::from(&list)
        .iter()
        .filter_map(|d| d.dyn_into::<MediaDeviceInfo>().ok())
        .filter(|d| d.kind() == want)
        .map(|d| DeviceInfo {
            id: d.device_id(),
            name: d.label(),
        })
        .collect()
}

#[doc(hidden)]
pub mod testing {
    //! Hooks for rinch-rtc's own tests (not API).

    /// Remote `<audio>` elements in the page, over every peer.
    pub fn remote_audio_elements() -> u32 {
        super::document()
            .and_then(|d| d.query_selector_all("[data-rinch-rtc-audio]").ok())
            .map(|l| l.length())
            .unwrap_or(0)
    }

    /// Local tracks whose browser track is held.
    pub fn live_local_tracks() -> usize {
        super::LOCAL_WEB.with(|l| l.borrow().len())
    }

    /// Frames a video source painted into its track.
    pub fn video_frames_taken(writer: &crate::media::VideoSourceWriter) -> u64 {
        writer.slot.taken.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Samples an audio source took off its queue.
    pub fn audio_samples_taken(writer: &crate::media::AudioSourceWriter) -> u64 {
        writer
            .queue
            .taken
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The browser track behind a remote track.
    pub fn remote_media_track(
        track: &crate::media::RemoteTrack,
    ) -> Option<web_sys::MediaStreamTrack> {
        super::with_peer(track.peer, |p| {
            p.remotes.get(&track.key).map(|r| Clone::clone(&r.track))
        })
        .flatten()
    }

    /// The id of the browser track `sender` carries.
    pub fn sender_track_id(sender: &crate::media::Sender) -> Option<String> {
        super::sender(sender.peer, sender.key)?
            .track()
            .map(|t| t.id())
    }

    /// The browser track behind a local track (to feed a test's own
    /// analysis).
    pub fn local_media_track(
        track: &crate::media::LocalTrack,
    ) -> Option<web_sys::MediaStreamTrack> {
        super::local_track(track.key())
    }
}
