//! Web capture lifetimes and failure paths (PR #1928's review): repeated
//! start/stop leaves no track, context, element or JS error behind; a setup
//! that fails releases the track and says so; the autoplay policy is
//! reported, not silent. Synthetic sources only (an OscillatorNode, a
//! canvas stream): no device, no permission prompt.
//!
//! `audio_status_says_whether_samples_arrive` runs twice in CI: with this
//! crate's webdriver.json (autoplay allowed: `Running`), and built with
//! `RINCH_AV_NO_AUTOPLAY=1` under a webdriver.json without the flag
//! (`WaitingForGesture`, no samples). That second run is Chrome's: Firefox
//! under automation runs the context even with `media.autoplay.default=1`
//! and `media.autoplay.block-webaudio=true` (measured, Firefox 1543 build).
#![cfg(target_arch = "wasm32")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_av::audio::{AudioInputConfig, AudioInputStatus, open_audio_input_from_stream};
use rinch_av::camera::{CameraConfig, open_camera_from_stream};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::{MediaStream, MediaStreamTrackState};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen(inline_js = r#"
export function rv_install() {
  if (window.__rv) return;
  window.__rv = { ctxs: [], procs: [], errors: [] };
  const B = BaseAudioContext.prototype;
  for (const m of ['createMediaStreamSource', 'createScriptProcessor']) {
    const o = B[m];
    B[m] = function(...a) {
      if (!window.__rv.ctxs.includes(this)) window.__rv.ctxs.push(this);
      const node = o.apply(this, a);
      if (m === 'createScriptProcessor') window.__rv.procs.push(node);
      return node;
    };
  }
  window.addEventListener('error', e => window.__rv.errors.push('err:' + String(e.message)));
  window.addEventListener('unhandledrejection', e => window.__rv.errors.push('rej:' + String(e.reason)));
}
export function rv_ctx_count() { return window.__rv.ctxs.length; }
export function rv_ctx_states(from) { return window.__rv.ctxs.slice(from).map(c => c.state).join(','); }
export function rv_errors() { return window.__rv.errors.join(' | '); }
export function rv_video_count() { return document.querySelectorAll('video').length; }
export function rv_ua() { return navigator.userAgent; }
// Firefox's order (#3007): a context delivers buffers before its `state`
// says `running` and before its `statechange` fires. While held, every
// AudioContext but `keep` reports `suspended` and its `statechange` handler
// calls wait; release restores `state` and then makes the waiting calls,
// which read `running`.
export function rv_hold_state(keep) {
  const B = BaseAudioContext.prototype;
  const state = Object.getOwnPropertyDescriptor(B, 'state').get;
  const on = Object.getOwnPropertyDescriptor(B, 'onstatechange');
  window.__rvHold = [];
  Object.defineProperty(AudioContext.prototype, 'state', {
    configurable: true,
    get() { return this === keep ? state.call(this) : 'suspended'; },
  });
  Object.defineProperty(AudioContext.prototype, 'onstatechange', {
    configurable: true,
    get() { return on.get.call(this); },
    set(h) {
      if (this === keep || !h) return on.set.call(this, h);
      const ctx = this;
      on.set.call(this, e => window.__rvHold ? window.__rvHold.push(() => h.call(ctx, e)) : h.call(ctx, e));
    },
  });
}
export function rv_release_state() {
  const waiting = window.__rvHold;
  window.__rvHold = null;
  delete AudioContext.prototype.state;
  delete AudioContext.prototype.onstatechange;
  if (window.__rvHeld) delete window.__rvHeld.state;
  window.__rvHeld = null;
  for (const f of waiting || []) f();
  return (waiting || []).length;
}
export function rv_suspend(i) { return window.__rv.ctxs[i].suspend(); }
export function rv_proc_count() { return window.__rv.procs.length; }
// A buffer the context queued before it stopped, reaching the handler late.
export function rv_late_buffer(i) {
  const p = window.__rv.procs[i];
  const b = new AudioBuffer({ length: 1024, sampleRate: p.context.sampleRate, numberOfChannels: 1 });
  p.onaudioprocess({ inputBuffer: b, outputBuffer: b, playbackTime: 0 });
}
export function rv_pointerdown() { window.dispatchEvent(new PointerEvent('pointerdown')); }
// Firefox's order after a resume: the context delivers before `state`
// reads `running` and before `statechange`. Context `i`'s `state` reads
// `suspended` and its installed `statechange` handler waits, until
// rv_release_state (which also lifts this instance override).
export function rv_hold_installed(i) {
  const ctx = window.__rv.ctxs[i];
  const h = ctx.onstatechange;
  window.__rvHold = [];
  window.__rvHeld = ctx;
  Object.defineProperty(ctx, 'state', { configurable: true, get() { return 'suspended'; } });
  ctx.onstatechange = e => window.__rvHold ? window.__rvHold.push(() => h.call(ctx, e)) : h.call(ctx, e);
}
// Context `i` reports `state` (until rv_release_state) and fires `statechange`.
export function rv_fake_state(i, state) {
  const ctx = window.__rv.ctxs[i];
  window.__rvHeld = ctx;
  Object.defineProperty(ctx, 'state', { configurable: true, get() { return state; } });
  ctx.dispatchEvent(new Event('statechange'));
}
export function rv_close(i) { return window.__rv.ctxs[i].close(); }
"#)]
extern "C" {
    fn rv_install();
    fn rv_ctx_count() -> u32;
    fn rv_ctx_states(from: u32) -> String;
    fn rv_errors() -> String;
    fn rv_video_count() -> u32;
    fn rv_ua() -> String;
    fn rv_hold_state(keep: &web_sys::AudioContext);
    fn rv_release_state() -> u32;
    fn rv_suspend(i: u32) -> js_sys::Promise;
    fn rv_proc_count() -> u32;
    fn rv_late_buffer(i: u32);
    fn rv_pointerdown();
    fn rv_hold_installed(i: u32);
    fn rv_fake_state(i: u32, state: &str);
    fn rv_close(i: u32) -> js_sys::Promise;
}

/// Resume `ctx`, waiting at most 3 s. A browser with no audio output can
/// leave `resume()` pending for ever; that must be a fast, named failure,
/// not a test the runner kills at its timeout.
async fn resume_within_3s(ctx: &web_sys::AudioContext) {
    let resume = ctx.resume().unwrap();
    let timeout = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3000);
    });
    let both: js_sys::Array = [JsValue::from(resume), JsValue::from(timeout)]
        .into_iter()
        .collect();
    let race = js_sys::Promise::race(&both);
    let _ = JsFuture::from(race).await;
    assert_eq!(
        ctx.state(),
        web_sys::AudioContextState::Running,
        "the test's AudioContext did not start within 3 s (state {:?}): this browser \
         has no working audio output (CI: is a sound server running?)",
        ctx.state()
    );
}

async fn sleep_ms(ms: i32) {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(p).await;
}

fn log(s: &str) {
    web_sys::console::log_1(&s.into());
}

fn first_track(stream: &MediaStream, audio: bool) -> web_sys::MediaStreamTrack {
    let list = if audio {
        stream.get_audio_tracks()
    } else {
        stream.get_video_tracks()
    };
    list.get(0).unchecked_into()
}

struct Tone {
    ctx: web_sys::AudioContext,
    osc: web_sys::OscillatorNode,
}

async fn tone_ctx(rate: Option<f32>) -> Tone {
    let ctx = match rate {
        Some(r) => {
            let o = web_sys::AudioContextOptions::new();
            o.set_sample_rate(r);
            web_sys::AudioContext::new_with_context_options(&o).unwrap()
        }
        None => web_sys::AudioContext::new().unwrap(),
    };
    let osc = ctx.create_oscillator().unwrap();
    osc.frequency().set_value(440.0);
    osc.start().unwrap();
    resume_within_3s(&ctx).await;
    Tone { ctx, osc }
}

fn tone_stream(t: &Tone) -> MediaStream {
    let dest = t.ctx.create_media_stream_destination().unwrap();
    t.osc.connect_with_audio_node(&dest).unwrap();
    dest.stream()
}

#[wasm_bindgen_test]
async fn audio_start_stop_50_leaves_nothing_behind() {
    rv_install();
    log(&format!("UA {}", rv_ua()));
    let tone = tone_ctx(None).await;
    log(&format!("tone ctx rate {}", tone.ctx.sample_rate()));
    let base = rv_ctx_count();
    let mut tracks = Vec::new();
    let calls = Arc::new(AtomicUsize::new(0));
    for i in 0..50 {
        let s = tone_stream(&tone);
        tracks.push(first_track(&s, true));
        let c = calls.clone();
        let input = open_audio_input_from_stream(
            &s,
            AudioInputConfig {
                sample_rate: 48_000,
                channels: 2,
                buffer_size: 256,
            },
            move |_| {
                c.fetch_add(1, Ordering::Relaxed);
            },
        );
        // Vary the drop point: before setup, just after, after callbacks.
        match i % 5 {
            0 => {}
            1 => sleep_ms(0).await,
            2 => sleep_ms(5).await,
            3 => sleep_ms(30).await,
            _ => sleep_ms(80).await,
        }
        drop(input);
    }
    sleep_ms(500).await;
    let after = calls.load(Ordering::Relaxed);
    sleep_ms(300).await;
    let live = tracks
        .iter()
        .filter(|t| t.ready_state() != MediaStreamTrackState::Ended)
        .count();
    let states = rv_ctx_states(base);
    let not_closed = states
        .split(',')
        .filter(|s| !s.is_empty() && *s != "closed")
        .count();
    log(&format!(
        "AUDIO50 live_tracks={live} ctxs_made={} not_closed={not_closed} states=[{states}] calls={after}->{} errors=[{}]",
        rv_ctx_count() - base,
        calls.load(Ordering::Relaxed),
        rv_errors()
    ));
    assert_eq!(live, 0, "live tracks after 50 start/stop");
    assert_eq!(not_closed, 0, "audio contexts left open: {states}");
    assert_eq!(after, calls.load(Ordering::Relaxed), "callbacks after drop");
    assert_eq!(rv_errors(), "", "uncaught errors");
}

/// A setup that fails after the stream arrives (the AudioContext refuses
/// the rate) releases the track at once, without waiting for the drop, and
/// reports the failure.
#[wasm_bindgen_test]
async fn audio_setup_failure_releases_the_track_and_says_why() {
    rv_install();
    let tone = tone_ctx(None).await;
    let s = tone_stream(&tone);
    let track = first_track(&s, true);
    let input = open_audio_input_from_stream(
        &s,
        AudioInputConfig {
            sample_rate: 1_000,
            channels: 1,
            buffer_size: 1024,
        },
        |_| {},
    );
    sleep_ms(200).await;
    assert_eq!(
        track.ready_state(),
        MediaStreamTrackState::Ended,
        "released before the drop"
    );
    assert!(
        matches!(input.status(), AudioInputStatus::Failed(_)),
        "{:?}",
        input.status()
    );
    drop(input);
}

/// Source stream from a 44.1 kHz context, capture asked for 48 kHz.
#[wasm_bindgen_test]
async fn audio_from_a_44k_source_into_a_48k_capture() {
    rv_install();
    let tone = tone_ctx(Some(44_100.0)).await;
    let s = tone_stream(&tone);
    let track = first_track(&s, true);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let input = open_audio_input_from_stream(
        &s,
        AudioInputConfig {
            sample_rate: 48_000,
            channels: 1,
            buffer_size: 1024,
        },
        move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        },
    );
    sleep_ms(800).await;
    let n = calls.load(Ordering::Relaxed);
    drop(input);
    sleep_ms(100).await;
    log(&format!(
        "MISMATCH calls={n} track after drop: {:?}",
        track.ready_state()
    ));
    assert_eq!(track.ready_state(), MediaStreamTrackState::Ended);
    assert!(n > 0, "callbacks with a 44.1k source into a 48k capture");
}

/// Left = tone, right = silence: is channel 1 really channel 1?
#[wasm_bindgen_test]
async fn audio_channels_are_interleaved_distinctly() {
    rv_install();
    let tone = tone_ctx(None).await;
    let merger = tone
        .ctx
        .create_channel_merger_with_number_of_inputs(2)
        .unwrap();
    tone.osc
        .connect_with_audio_node_and_output_and_input(&merger, 0, 0)
        .unwrap();
    let dest = tone.ctx.create_media_stream_destination().unwrap();
    merger.connect_with_audio_node(&dest).unwrap();
    let s = dest.stream();
    let l = Arc::new(AtomicUsize::new(0));
    let r = Arc::new(AtomicUsize::new(0));
    let (l2, r2) = (l.clone(), r.clone());
    let input = open_audio_input_from_stream(
        &s,
        AudioInputConfig {
            sample_rate: 48_000,
            channels: 2,
            buffer_size: 1024,
        },
        move |samples: &[f32]| {
            let el: f32 = samples.iter().step_by(2).map(|x| x * x).sum();
            let er: f32 = samples.iter().skip(1).step_by(2).map(|x| x * x).sum();
            if el > 1.0 {
                l2.fetch_add(1, Ordering::Relaxed);
            }
            if er > 1.0 {
                r2.fetch_add(1, Ordering::Relaxed);
            }
        },
    );
    sleep_ms(800).await;
    log(&format!(
        "INTERLEAVE loud_left={} loud_right={}",
        l.load(Ordering::Relaxed),
        r.load(Ordering::Relaxed)
    ));
    assert!(l.load(Ordering::Relaxed) > 0);
    assert_eq!(r.load(Ordering::Relaxed), 0, "right channel is silent");
    drop(input);
}

fn canvas_stream() -> MediaStream {
    let doc = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement =
        doc.create_element("canvas").unwrap().dyn_into().unwrap();
    canvas.set_width(32);
    canvas.set_height(24);
    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    ctx.set_fill_style_str("rgb(0,0,255)");
    ctx.fill_rect(0.0, 0.0, 32.0, 24.0);
    canvas.capture_stream_with_frame_request_rate(30.0).unwrap()
}

#[wasm_bindgen_test]
async fn camera_start_stop_50_leaves_nothing_behind() {
    rv_install();
    let v0 = rv_video_count();
    let mut tracks = Vec::new();
    let frames = Arc::new(AtomicUsize::new(0));
    for i in 0..50 {
        let s = canvas_stream();
        tracks.push(first_track(&s, false));
        let f = frames.clone();
        let mut cap = open_camera_from_stream(
            &s,
            CameraConfig {
                width: 32,
                height: 24,
                ..Default::default()
            },
            move |_| {
                f.fetch_add(1, Ordering::Relaxed);
            },
        );
        match i % 6 {
            0 => {}
            1 => {
                // one microtask: after the stream resolves, during play()
                let _ = JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
            }
            2 => sleep_ms(0).await,
            3 => sleep_ms(5).await,
            4 => sleep_ms(30).await,
            _ => sleep_ms(100).await,
        }
        if i % 2 == 0 {
            cap.stop();
            drop(cap);
        } else {
            drop(cap);
        }
    }
    sleep_ms(800).await;
    let live = tracks
        .iter()
        .filter(|t| t.ready_state() != MediaStreamTrackState::Ended)
        .count();
    let after = frames.load(Ordering::Relaxed);
    sleep_ms(300).await;
    log(&format!(
        "CAMERA50 live_tracks={live} videos_left={} frames={after}->{} errors=[{}]",
        rv_video_count() - v0,
        frames.load(Ordering::Relaxed),
        rv_errors()
    ));
    assert_eq!(live, 0);
    assert_eq!(rv_video_count() - v0, 0, "hidden <video> elements leaked");
    assert_eq!(after, frames.load(Ordering::Relaxed), "frames after stop");
    assert_eq!(rv_errors(), "", "uncaught errors");
}

/// The capture context's state is reported. A browser that allows
/// autoplay (this crate's webdriver.json) runs it; one that does not holds
/// it until a gesture, and the status says so instead of delivering
/// silence with no error.
#[wasm_bindgen_test]
async fn audio_status_says_whether_samples_arrive() {
    rv_install();
    let no_autoplay = option_env!("RINCH_AV_NO_AUTOPLAY").is_some();
    let ctx = web_sys::AudioContext::new().unwrap();
    let osc = ctx.create_oscillator().unwrap();
    osc.start().unwrap();
    let dest = ctx.create_media_stream_destination().unwrap();
    osc.connect_with_audio_node(&dest).unwrap();
    let s = dest.stream();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let input = open_audio_input_from_stream(&s, AudioInputConfig::default(), move |_| {
        c.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(input.status(), AudioInputStatus::Starting);
    // A context that will run may take a while to open the output (real
    // Firefox's first stream on a sound server); one that is held stays
    // held. Wait up to 5 s for samples, then judge.
    for _ in 0..50 {
        sleep_ms(100).await;
        if calls.load(Ordering::Relaxed) > 0 {
            break;
        }
    }
    let n = calls.load(Ordering::Relaxed);
    let status = input.status();
    log(&format!(
        "AUTOPLAY no_autoplay={no_autoplay} status={status:?} calls={n} errors=[{}]",
        rv_errors()
    ));
    if no_autoplay {
        assert_eq!(status, AudioInputStatus::WaitingForGesture);
        assert_eq!(n, 0, "a held context delivers nothing");
    } else {
        assert_eq!(status, AudioInputStatus::Running);
        assert!(n > 0, "a running context delivers samples");
    }
    drop(input);
    let _ = ctx.close();
}

/// Samples delivered mean `Running`, whatever the context's `state` says
/// (#3007). Firefox delivers a context's first buffers (rendered on its
/// fallback clock while the audio output opens) before `state` reads
/// `running` and before `statechange` fires; the status, derived from
/// `state` alone, said `WaitingForGesture` with samples already in hand.
/// That order is made here in any browser: every context but the source's
/// reports `suspended` and its `statechange` waits, so the capture context
/// delivers while its `state` says it is held and before its `statechange`.
///
/// The no-autoplay run skips it: a held context delivers nothing, and
/// `audio_status_says_whether_samples_arrive` pins that.
#[wasm_bindgen_test]
async fn audio_is_running_once_samples_arrive_whatever_state_says() {
    if option_env!("RINCH_AV_NO_AUTOPLAY").is_some() {
        return;
    }
    rv_install();
    let tone = tone_ctx(None).await;
    let s = tone_stream(&tone);
    rv_hold_state(&tone.ctx);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let input = open_audio_input_from_stream(&s, AudioInputConfig::default(), move |_| {
        c.fetch_add(1, Ordering::Relaxed);
    });
    for _ in 0..50 {
        sleep_ms(100).await;
        if calls.load(Ordering::Relaxed) > 0 {
            break;
        }
    }
    let n = calls.load(Ordering::Relaxed);
    let status = input.status();
    // The `statechange` that waited, now reading `running`.
    let waited = rv_release_state();
    let after = input.status();
    drop(input);
    let _ = tone.ctx.close();
    assert!(n > 0, "the capture context delivers samples");
    assert_eq!(
        status,
        AudioInputStatus::Running,
        "{n} buffers delivered, yet the status says the context is held"
    );
    assert_eq!(waited, 1, "the capture context's statechange waited");
    assert_eq!(after, AudioInputStatus::Running);
}

/// Wait up to 3 s for `f`.
async fn until(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..30 {
        if f() {
            return true;
        }
        sleep_ms(100).await;
    }
    f()
}

/// A context that stops after running is held again: the status says so
/// (a buffer queued before it stopped does not say `Running`), and the
/// page's next gesture resumes it. The no-autoplay run skips it (nothing
/// runs there).
#[wasm_bindgen_test]
async fn audio_stopped_after_running_waits_for_a_gesture_again() {
    if option_env!("RINCH_AV_NO_AUTOPLAY").is_some() {
        return;
    }
    rv_install();
    let tone = tone_ctx(None).await;
    let (input, calls, capture, processor) = running_input(&tone_stream(&tone)).await;

    // The context stops (here by `suspend()`; the browser can do it too).
    let _ = JsFuture::from(rv_suspend(capture)).await;
    let held = until(|| input.status() == AudioInputStatus::WaitingForGesture).await;
    // A buffer queued before it stopped arrives late: the status stays held.
    let delivered = calls.load(Ordering::Relaxed);
    rv_late_buffer(processor);
    let late = calls.load(Ordering::Relaxed) - delivered;
    let still = input.status();

    // A gesture resumes it, and samples arrive again.
    let before = calls.load(Ordering::Relaxed);
    rv_pointerdown();
    let resumed = until(|| {
        input.status() == AudioInputStatus::Running && calls.load(Ordering::Relaxed) > before
    })
    .await;
    let status = input.status();
    drop(input);
    let _ = tone.ctx.close();
    assert!(held, "a stopped context says it is held");
    assert_eq!(late, 1, "the late buffer reached the capture's handler");
    assert_eq!(still, AudioInputStatus::WaitingForGesture);
    assert!(resumed, "the gesture resumed capture (status {status:?})");
}

/// Open capture from `s` and wait until it delivers. Returns the input,
/// its call counter, and the indices of its context and processor.
async fn running_input(
    s: &MediaStream,
) -> (
    rinch_av::audio::AudioInputStream,
    Arc<AtomicUsize>,
    u32,
    u32,
) {
    let capture = rv_ctx_count();
    let processor = rv_proc_count();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let input = open_audio_input_from_stream(s, AudioInputConfig::default(), move |_| {
        c.fetch_add(1, Ordering::Relaxed);
    });
    assert!(
        until(|| calls.load(Ordering::Relaxed) > 0).await,
        "the capture context delivers samples"
    );
    assert_eq!(input.status(), AudioInputStatus::Running);
    (input, calls, capture, processor)
}

/// Resumed by a gesture, a stopped context's first buffers arrive (in
/// Firefox) while `state` still reads `suspended` and before `statechange`
/// says `running`: they are the resumed context's, so the status says
/// `Running` as they arrive (#3008's review). Made here in any browser by
/// holding the installed handler and the context's `state`.
#[wasm_bindgen_test]
async fn audio_resumed_by_a_gesture_is_running_once_samples_arrive() {
    if option_env!("RINCH_AV_NO_AUTOPLAY").is_some() {
        return;
    }
    rv_install();
    let tone = tone_ctx(None).await;
    let (input, calls, capture, _) = running_input(&tone_stream(&tone)).await;
    let _ = JsFuture::from(rv_suspend(capture)).await;
    let held = until(|| input.status() == AudioInputStatus::WaitingForGesture).await;

    rv_hold_installed(capture);
    let before = calls.load(Ordering::Relaxed);
    rv_pointerdown();
    let delivered = until(|| calls.load(Ordering::Relaxed) > before).await;
    let status = input.status();
    let waited = rv_release_state();
    let after = input.status();
    drop(input);
    let _ = tone.ctx.close();
    assert!(held, "a stopped context says it is held");
    assert!(delivered, "the gesture resumed delivery");
    assert_eq!(
        status,
        AudioInputStatus::Running,
        "samples delivered after the gesture's resume, yet the status says held"
    );
    assert_eq!(waited, 1, "the resume's statechange waited");
    assert_eq!(after, AudioInputStatus::Running);
}

/// Safari's `interrupted` stops delivery as `suspended` does: the status
/// says held (it said `Running`), and `running` again says `Running`.
#[wasm_bindgen_test]
async fn audio_interrupted_says_held() {
    if option_env!("RINCH_AV_NO_AUTOPLAY").is_some() {
        return;
    }
    rv_install();
    let tone = tone_ctx(None).await;
    let (input, _, capture, processor) = running_input(&tone_stream(&tone)).await;
    rv_fake_state(capture, "interrupted");
    let interrupted = input.status();
    // A buffer queued before the interruption does not say `Running`.
    rv_late_buffer(processor);
    let late = input.status();
    rv_fake_state(capture, "running");
    let back = input.status();
    rv_release_state();
    drop(input);
    let _ = tone.ctx.close();
    assert_eq!(interrupted, AudioInputStatus::WaitingForGesture);
    assert_eq!(late, AudioInputStatus::WaitingForGesture);
    assert_eq!(back, AudioInputStatus::Running);
}

/// The capture context closed under the stream (not by its drop): nothing
/// will arrive again, so the status says `Failed`, the track is stopped,
/// and a buffer still queued does not say `Running`.
#[wasm_bindgen_test]
async fn audio_context_closed_under_the_stream_fails_and_ends_the_track() {
    if option_env!("RINCH_AV_NO_AUTOPLAY").is_some() {
        return;
    }
    rv_install();
    let tone = tone_ctx(None).await;
    let s = tone_stream(&tone);
    let track: web_sys::MediaStreamTrack = s.get_audio_tracks().get(0).unchecked_into();
    let (input, _, capture, processor) = running_input(&s).await;
    let _ = JsFuture::from(rv_close(capture)).await;
    let failed = until(|| matches!(input.status(), AudioInputStatus::Failed(_))).await;
    rv_late_buffer(processor);
    let late = input.status();
    let ended = track.ready_state();
    drop(input);
    let _ = tone.ctx.close();
    assert!(failed, "the status after the close: {late:?}");
    assert!(matches!(late, AudioInputStatus::Failed(_)), "{late:?}");
    assert_eq!(ended, MediaStreamTrackState::Ended);
}
