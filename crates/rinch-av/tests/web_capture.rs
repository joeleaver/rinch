//! rinch-av's web capture against synthetic sources (issue #1916, design
//! record rtc-media.md 3.5): an `OscillatorNode` stands in for a microphone
//! and a canvas stream for a camera. Nothing here opens a real device.
#![cfg(target_arch = "wasm32")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_av::audio::{AudioInputConfig, open_audio_input_from_stream};
use rinch_av::camera::{CameraConfig, open_camera_from_stream};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::{MediaStream, MediaStreamTrackState};

wasm_bindgen_test_configure!(run_in_browser);

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

/// A 440 Hz tone as a MediaStream (and the context that keeps it running).
async fn tone() -> (web_sys::AudioContext, MediaStream) {
    let ctx = web_sys::AudioContext::new().unwrap();
    let osc = ctx.create_oscillator().unwrap();
    osc.frequency().set_value(440.0);
    let dest = ctx.create_media_stream_destination().unwrap();
    osc.connect_with_audio_node(&dest).unwrap();
    osc.start().unwrap();
    resume_within_3s(&ctx).await;
    (ctx, dest.stream())
}

/// A canvas painted solid red, as a 30 fps MediaStream.
fn red_canvas_stream() -> MediaStream {
    let doc = web_sys::window().unwrap().document().unwrap();
    let canvas: web_sys::HtmlCanvasElement =
        doc.create_element("canvas").unwrap().dyn_into().unwrap();
    canvas.set_width(64);
    canvas.set_height(48);
    let ctx: web_sys::CanvasRenderingContext2d = canvas
        .get_context("2d")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    ctx.set_fill_style_str("rgb(255,0,0)");
    ctx.fill_rect(0.0, 0.0, 64.0, 48.0);
    // Repaint every 33 ms, as a live camera keeps producing frames (Chrome
    // emits a canvas-stream frame only when the canvas is painted).
    let repaint = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        ctx.fill_rect(0.0, 0.0, 64.0, 48.0);
    });
    web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            repaint.as_ref().unchecked_ref(),
            33,
        )
        .unwrap();
    repaint.forget();
    canvas.capture_stream_with_frame_request_rate(30.0).unwrap()
}

fn first_track(stream: &MediaStream, audio: bool) -> web_sys::MediaStreamTrack {
    let list = if audio {
        stream.get_audio_tracks()
    } else {
        stream.get_video_tracks()
    };
    list.get(0).unchecked_into()
}

#[wasm_bindgen_test]
async fn audio_input_keeps_delivering_after_setup_and_its_drop_ends_the_track() {
    let (_ctx, stream) = tone().await;
    let track = first_track(&stream, true);
    let calls = Arc::new(AtomicUsize::new(0));
    let loud = Arc::new(AtomicUsize::new(0));
    let frames = Arc::new(AtomicUsize::new(0));
    let (c, l, f) = (calls.clone(), loud.clone(), frames.clone());
    let input = open_audio_input_from_stream(
        &stream,
        AudioInputConfig {
            sample_rate: 48_000,
            channels: 2,
            buffer_size: 1024,
        },
        move |samples: &[f32]| {
            c.fetch_add(1, Ordering::Relaxed);
            f.store(samples.len(), Ordering::Relaxed);
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
            if rms > 0.05 {
                l.fetch_add(1, Ordering::Relaxed);
            }
        },
    );
    // Setup finishes within the first moments; the callbacks must keep coming
    // well after it (the state used to be dropped once setup ended).
    sleep_ms(600).await;
    let early = calls.load(Ordering::Relaxed);
    sleep_ms(900).await;
    let late = calls.load(Ordering::Relaxed);
    assert!(early > 0, "the tone reached the callback");
    assert!(
        late > early + 5,
        "callbacks continue after setup: {early} then {late}"
    );
    assert!(loud.load(Ordering::Relaxed) > 0, "and they carry the tone");
    assert_eq!(
        frames.load(Ordering::Relaxed),
        2 * 1024,
        "two channels, interleaved"
    );

    drop(input);
    sleep_ms(50).await;
    assert_eq!(track.ready_state(), MediaStreamTrackState::Ended);
    let stopped = calls.load(Ordering::Relaxed);
    sleep_ms(300).await;
    assert_eq!(
        calls.load(Ordering::Relaxed),
        stopped,
        "nothing after the drop"
    );
}

#[wasm_bindgen_test]
async fn audio_input_dropped_before_setup_still_ends_the_track() {
    let (_ctx, stream) = tone().await;
    let track = first_track(&stream, true);
    drop(open_audio_input_from_stream(
        &stream,
        AudioInputConfig::default(),
        |_| {},
    ));
    sleep_ms(100).await;
    assert_eq!(track.ready_state(), MediaStreamTrackState::Ended);
}

#[wasm_bindgen_test]
async fn camera_capture_delivers_frames_and_stop_ends_the_track() {
    let stream = red_canvas_stream();
    let track = first_track(&stream, false);
    let frames = Arc::new(AtomicUsize::new(0));
    let red = Arc::new(AtomicUsize::new(0));
    let (f, r) = (frames.clone(), red.clone());
    let mut cap = open_camera_from_stream(
        &stream,
        CameraConfig {
            width: 64,
            height: 48,
            ..Default::default()
        },
        move |frame| {
            f.fetch_add(1, Ordering::Relaxed);
            if frame.data[0] > 200 && frame.data[1] < 50 && frame.data[2] < 50 {
                r.fetch_add(1, Ordering::Relaxed);
            }
        },
    );
    for _ in 0..100 {
        if red.load(Ordering::Relaxed) > 0 {
            break;
        }
        sleep_ms(20).await;
    }
    assert!(red.load(Ordering::Relaxed) > 0, "the canvas's red arrived");
    // The hidden preview is muted by property, which is what lets a browser
    // play it without a user gesture (the CI Chrome allows autoplay anyway,
    // so the frames alone would not show it).
    let videos = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .query_selector_all("video")
        .unwrap();
    assert!(videos.length() > 0);
    for i in 0..videos.length() {
        let v: web_sys::HtmlMediaElement = videos.get(i).unwrap().unchecked_into();
        assert!(v.muted(), "the camera's preview video is muted");
    }
    cap.stop();
    assert_eq!(track.ready_state(), MediaStreamTrackState::Ended);
}

#[wasm_bindgen_test]
async fn camera_stopped_before_its_stream_arrives_still_ends_the_track() {
    let stream = red_canvas_stream();
    let track = first_track(&stream, false);
    let frames = Arc::new(AtomicUsize::new(0));
    let f = frames.clone();
    let mut cap = open_camera_from_stream(&stream, CameraConfig::default(), move |_| {
        f.fetch_add(1, Ordering::Relaxed);
    });
    // Before the promise resolves (a microtask away).
    cap.stop();
    sleep_ms(200).await;
    assert_eq!(track.ready_state(), MediaStreamTrackState::Ended);
    assert_eq!(frames.load(Ordering::Relaxed), 0);
}
