//! An offer whose video lists VP8 alone (#1975's review, finding 2).
//!
//! Without `media-vp8` the answer rejects that video m-line (port 0): the
//! call connects on whatever else the offer carried, and webrtc-rs says
//! nothing. rinch-rtc then reports it through `on_error`, naming VP8 and
//! the `media-vp8` feature, whether or not the answerer has a video track of
//! its own. With `media-vp8` the same offer negotiates VP8 and nothing is
//! reported. Synthetic sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::media::{Pair, moving_box, tone};
use common::{T, drive, init};
use rinch_rtc::Description;
use rinch_rtc::media::{AudioSourceOptions, LocalTrack, VideoSourceOptions};

/// Every video m-line's payloads replaced by VP8 alone on PT 96.
fn vp8_only(sdp: &str) -> String {
    let mut out = Vec::new();
    let mut in_video = false;
    let mut added = false;
    for l in sdp.split("\r\n").filter(|l| !l.is_empty()) {
        if l.starts_with("m=") {
            in_video = l.starts_with("m=video");
            added = false;
            if in_video {
                let m: Vec<&str> = l.split(' ').collect();
                out.push(format!("{} 96", m[..3].join(" ")));
                continue;
            }
        } else if in_video
            && ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"]
                .iter()
                .any(|p| l.starts_with(p))
        {
            if !added {
                out.push("a=rtpmap:96 VP8/90000".into());
                out.push("a=rtcp-fb:96 nack".into());
                out.push("a=rtcp-fb:96 nack pli".into());
                added = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    out.join("\r\n") + "\r\n"
}

/// The two shapes in turn (the peers share process-wide state): an offer
/// with audio and video to an answerer with no track, and a video-only
/// offer to an answerer that adds its own video track.
#[test]
fn a_vp8_only_offer_is_reported_without_media_vp8() {
    go(true);
    go(false);
}

fn go(audio: bool) {
    init();
    let run = Arc::new(AtomicBool::new(true));
    let opts = VideoSourceOptions {
        width: 320,
        height: 240,
        fps: 30,
    };
    let (va, wa) = LocalTrack::video_source(opts).unwrap();
    moving_box(wa, 320, 240, 64, run.clone());
    let (vb, wb) = LocalTrack::video_source(opts).unwrap();
    moving_box(wb, 320, 240, 64, run.clone());
    let pair = Pair::new();
    let errs: Rc<RefCell<Vec<String>>> = Rc::default();
    let e = errs.clone();
    pair.b
        .on_error(move |x| e.borrow_mut().push(format!("{x}")));
    pair.a.add_track(&va).unwrap();
    let mut keep = Vec::new();
    if audio {
        let (at, aw) = LocalTrack::audio_source(AudioSourceOptions::default()).unwrap();
        tone(aw, 48000, run.clone());
        pair.a.add_track(&at).unwrap();
        keep.push(at);
    }
    let offer = drive(pair.a.create_offer(), T).unwrap();
    let munged = vp8_only(&offer.sdp);
    assert!(munged.contains("a=rtpmap:96 VP8/90000"), "{munged}");
    drive(pair.b.set_remote_description(Description::offer(munged)), T).unwrap();
    if !audio {
        pair.b.add_track(&vb).unwrap();
    }
    let answer = drive(pair.b.create_answer(), T).unwrap();
    pair.wait(500);
    let errs = errs.borrow().clone();
    let reported: Vec<&String> = errs
        .iter()
        .filter(|e| e.contains("VP8") && e.contains("`media-vp8`"))
        .collect();
    if cfg!(feature = "media-vp8") {
        assert!(
            !answer.sdp.contains("m=video 0 "),
            "VP8 is answered: {}",
            answer.sdp
        );
        assert!(reported.is_empty(), "nothing to report: {errs:?}");
    } else {
        assert!(
            answer.sdp.contains("m=video 0 "),
            "the video m-line is rejected: {}",
            answer.sdp
        );
        assert_eq!(
            reported.len(),
            1,
            "one error naming VP8 and `media-vp8` (audio {audio}): {errs:?}"
        );
        assert!(
            reported[0].contains("rejected") && reported[0].contains("H264"),
            "says what happened and what this build has: {}",
            reported[0]
        );
    }
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);
    drop(keep);
}
