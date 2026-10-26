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
  window.__rv = { ctxs: [], errors: [] };
  const B = BaseAudioContext.prototype;
  for (const m of ['createMediaStreamSource', 'createScriptProcessor']) {
    const o = B[m];
    B[m] = function(...a) { if (!window.__rv.ctxs.includes(this)) window.__rv.ctxs.push(this); return o.apply(this, a); };
  }
  window.addEventListener('error', e => window.__rv.errors.push('err:' + String(e.message)));
  window.addEventListener('unhandledrejection', e => window.__rv.errors.push('rej:' + String(e.reason)));
}
export function rv_ctx_count() { return window.__rv.ctxs.length; }
export function rv_ctx_states(from) { return window.__rv.ctxs.slice(from).map(c => c.state).join(','); }
export function rv_errors() { return window.__rv.errors.join(' | '); }
export function rv_video_count() { return document.querySelectorAll('video').length; }
export function rv_ua() { return navigator.userAgent; }
"#)]
extern "C" {
    fn rv_install();
    fn rv_ctx_count() -> u32;
    fn rv_ctx_states(from: u32) -> String;
    fn rv_errors() -> String;
    fn rv_video_count() -> u32;
    fn rv_ua() -> String;
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
    let _ = JsFuture::from(ctx.resume().unwrap()).await;
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
    sleep_ms(800).await;
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
