//! Two of this example's calls meeting through a real `rtc-room-relay` on
//! loopback, with no window (#1916): the signalling protocol, who offers,
//! the pattern arriving at the far side, and "bye". No device is asked for:
//! "Start camera" is never pressed, and nothing else opens one.
//!
//! The test thread stands in for the rinch runtime, as in rinch-rtc's and
//! rinch-ws's own tests: it registers as the main thread and pumps the
//! main-thread queue. Registration is process-global: one `#[test]`.

use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};

use super::*;

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
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn remote_video(call: &Call) -> rinch_rtc::media::RemoteTrack {
    match &call.ui.remote.get().first().expect("a remote video").0 {
        MediaTrack::Remote(t) => t.clone(),
        other => panic!("not a remote track: {other:?}"),
    }
}

/// Frames whose bright part is the pattern's box (96 px a side).
fn box_frames(track: &rinch_rtc::media::RemoteTrack) -> Arc<Mutex<(u32, u32)>> {
    let seen: Arc<Mutex<(u32, u32)>> = Arc::default();
    let s = seen.clone();
    track.on_frame(move |rgba, w, h| {
        let (mut x0, mut x1, mut y0, mut y1, mut n) = (u32::MAX, 0, u32::MAX, 0, 0u32);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if rgba[i] > 160 && rgba[i + 1] > 160 && rgba[i + 2] > 160 {
                    n += 1;
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                }
            }
        }
        let near = |v: u32| v.abs_diff(96) <= 24;
        let is_box = n > 0 && near(x1 - x0 + 1) && near(y1 - y0 + 1);
        let mut s = s.lock().unwrap();
        s.0 += 1;
        s.1 += u32::from(is_box);
    });
    seen
}

#[test]
fn two_calls_meet_through_the_relay_and_see_each_others_pattern() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
    let _no_speaker = rinch_rtc::testing::capture_audio_output();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://127.0.0.1:{}", listener.local_addr().unwrap().port());
    std::thread::spawn(move || rtc_room_relay::run(listener));

    let first = new_call();
    let second = new_call();
    for call in [&first, &second] {
        call.ui.server_url.set(url.clone());
        call.ui.room.set("video-call-test".into());
    }
    first.join();
    pump_until("the first to reach the relay", || {
        first.ui.status.get().starts_with("Waiting")
    });
    second.join();
    pump_until("both peers connected", || {
        first.ui.state.get() == PeerState::Connected
            && second.ui.state.get() == PeerState::Connected
    });
    // The relay told the first that the second joined: the first offered.
    assert!(!first.polite.get(), "the waiting side offers");
    assert!(second.polite.get(), "the newcomer is the polite side");

    // Each side's pattern reaches the other.
    pump_until("both remote videos", || {
        !first.ui.remote.get().is_empty() && !second.ui.remote.get().is_empty()
    });
    let at_second = box_frames(&remote_video(&second));
    let at_first = box_frames(&remote_video(&first));
    pump_until("frames both ways", || {
        at_second.lock().unwrap().0 >= 15 && at_first.lock().unwrap().0 >= 15
    });
    for (who, seen) in [("second", &at_second), ("first", &at_first)] {
        let (frames, boxes) = *seen.lock().unwrap();
        assert!(
            boxes * 10 >= frames * 8,
            "the box in {boxes} of {frames} frames at the {who}"
        );
    }

    // Nothing asked for a device; both still send their pattern.
    for call in [&first, &second] {
        assert!(call.camera.borrow().is_none() && call.mic.borrow().is_none());
        let sent = call.video.borrow().as_ref().and_then(|s| s.track());
        assert_eq!(sent.map(|t| t.id()), Some(call.pattern.id()));
    }

    // The second leaves: the first hears "bye" and waits with a fresh peer.
    second.leave();
    pump_until("the first to hear bye", || {
        first.ui.status.get().contains("left") && first.ui.remote.get().is_empty()
    });
    assert_eq!(first.ui.state.get(), PeerState::New);
    assert!(first.polite.get(), "after bye, the next arrival offers");
    first.leave();
}
