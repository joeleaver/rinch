//! The WebRTC section's "Use camera" when the senders cannot take the
//! devices (#1916, review of PR #1970 F5): the devices are released, the
//! pattern goes back on the senders, and the button is usable again. Before,
//! the devices stayed open and `source` stayed `Requesting`, so "Use camera"
//! stayed disabled and nothing released them until the section unmounted.
//!
//! Synthetic tracks stand in for the camera and microphone
//! (`InPageCall::use_devices`, the seam `use_camera` goes through); no device
//! is opened. A stopped "microphone" makes the audio sender's
//! `replace_track` fail after the video sender has already taken the
//! "camera": the partial case.
//!
//! The test thread stands in for the rinch runtime, as in
//! `webrtc_in_page.rs`. Registration is process-global, so this file has one
//! `#[test]`.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_rtc::RtcError;
use rinch_rtc::media::{AudioSourceOptions, LocalTrack, VideoSourceOptions};
use ui_zoo::sections::webrtc_call::{InPageCall, Source};

type MainJob = Box<dyn FnOnce() + Send>;
static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

fn dispatcher(f: MainJob) {
    PUMP.get().expect("pump").lock().unwrap().push(f);
}

fn drain() {
    loop {
        let jobs: Vec<_> = PUMP.get().unwrap().lock().unwrap().drain(..).collect();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            job();
        }
    }
}

fn pump_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        drain();
        if cond() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn fake_devices() -> (LocalTrack, LocalTrack) {
    let (camera, _) = LocalTrack::video_source(VideoSourceOptions {
        width: 64,
        height: 48,
        fps: 30,
    })
    .unwrap();
    let (mic, _) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
    (camera, mic)
}

fn sending_id(call: &InPageCall) -> Option<String> {
    call.sending().map(|t| t.id())
}

#[test]
fn a_camera_the_senders_refuse_is_released() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
    let _no_speaker = rinch_rtc::testing::capture_audio_output();

    let call = InPageCall::start().expect("the call starts");
    let view = call.view();
    let pattern = sending_id(&call).expect("A sends the pattern");

    // Positive control: devices the senders take are sent, and released
    // by "Use pattern".
    let (camera, mic) = fake_devices();
    let (c, m) = (camera.clone(), mic.clone());
    call.use_devices(async move { Ok::<_, RtcError>((c, m)) });
    pump_until("the camera is sent", || {
        view.source.get() != Source::Requesting
    });
    assert_eq!(
        view.source.get(),
        Source::Camera,
        "error: {}",
        view.error.get()
    );
    assert_eq!(sending_id(&call), Some(camera.id()));
    assert!(!camera.ended() && !mic.ended());
    call.use_pattern();
    pump_until("back to the pattern", || camera.ended());
    assert!(mic.ended(), "use_pattern releases the microphone");
    assert_eq!(view.source.get(), Source::Pattern);
    assert_eq!(sending_id(&call), Some(pattern.clone()));

    // The audio sender refuses the "microphone" (already ended), after the
    // video sender has taken the "camera".
    let (camera, mic) = fake_devices();
    mic.stop();
    let (c, m) = (camera.clone(), mic.clone());
    call.use_devices(async move { Ok::<_, RtcError>((c, m)) });
    pump_until("the swap settles", || {
        view.source.get() != Source::Requesting
    });
    assert_eq!(
        view.source.get(),
        Source::Pattern,
        "a failed swap goes back to the pattern, so the button works again"
    );
    assert!(
        camera.ended(),
        "the camera the senders did not keep is released"
    );
    assert!(
        view.error.get().contains("replace_track"),
        "the error is shown: {:?}",
        view.error.get()
    );
    assert_eq!(
        sending_id(&call),
        Some(pattern.clone()),
        "the pattern is back on the video sender"
    );
    let shown: Vec<String> = view.local.get().iter().map(|s| s.id()).collect();
    assert_eq!(shown, vec![pattern], "the self-view shows the pattern");

    call.close();
}
