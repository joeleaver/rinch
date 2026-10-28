//! Codec negotiation on desktop (#1947's second review, adopted): a desktop
//! peer offers its codecs on every video m-line in its preferred order (VP8
//! and H.264 with `media-vp8`, H.264 alone without it), answers whatever
//! subset an offer lists (never a codec the offer did not list), sends in the
//! codec the answer picked, and follows a change of codec in a
//! renegotiation. Without `media-vp8`, a browser's VP8-first offer is
//! answered in H.264. Synthetic sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::media::{Pair, is_box, moving_box};
use common::{T, drive, init};
use rinch_rtc::media::{LocalTrack, RemoteTrack, VideoSourceOptions};
use rinch_rtc::{Description, testing};

/// The encoding names of the first video m-line's payload types, in order
/// (RTX left out).
fn video_codecs(sdp: &str) -> Vec<String> {
    let mut pts: Vec<String> = Vec::new();
    let mut names = std::collections::HashMap::new();
    let mut in_video = false;
    for l in sdp.lines() {
        if l.starts_with("m=") {
            if !pts.is_empty() {
                break;
            }
            in_video = l.starts_with("m=video");
            if in_video {
                pts = l.split(' ').skip(3).map(String::from).collect();
            }
        } else if in_video && let Some(r) = l.strip_prefix("a=rtpmap:") {
            let (pt, name) = r.split_once(' ').unwrap();
            names.insert(pt.to_string(), name.split('/').next().unwrap().to_string());
        }
    }
    pts.iter()
        .filter_map(|p| names.get(p).cloned())
        .filter(|n| !n.eq_ignore_ascii_case("rtx"))
        .collect()
}

/// `sdp` with VP8 put first on its video m-lines (payload type 96, unused by
/// a build without VP8): the shape of every browser's offer.
#[cfg(not(feature = "media-vp8"))]
fn with_vp8_first(sdp: &str) -> String {
    assert!(!sdp.contains("a=rtpmap:96 "), "96 is free");
    let mut out = Vec::new();
    let mut pending = false;
    for l in sdp.split("\r\n").filter(|l| !l.is_empty()) {
        if l.starts_with("m=") {
            pending = l.starts_with("m=video");
            if pending {
                let m: Vec<&str> = l.split(' ').collect();
                out.push(format!("{} 96 {}", m[..3].join(" "), m[3..].join(" ")));
                continue;
            }
        } else if pending && l.starts_with("a=rtpmap:") {
            out.push("a=rtpmap:96 VP8/90000".to_string());
            pending = false;
        }
        out.push(l.to_string());
    }
    out.join("\r\n") + "\r\n"
}

/// `sdp` with its video m-lines cut to `keep` (codec names, in that order)
/// and their RTX: what an H.264-only or H.264-first browser would offer.
fn only(sdp: &str, keep: &[&str]) -> String {
    let lines: Vec<&str> = sdp.split("\r\n").filter(|l| !l.is_empty()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("m=video") {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let end = (i + 1..lines.len())
            .find(|&j| lines[j].starts_with("m="))
            .unwrap_or(lines.len());
        let sec = &lines[i + 1..end];
        let name_of = |pt: &str| {
            sec.iter()
                .find_map(|l| l.strip_prefix(&format!("a=rtpmap:{pt} ")))
                .map(|r| r.split('/').next().unwrap().to_string())
        };
        let apt_of = |pt: &str| {
            sec.iter()
                .find_map(|l| l.strip_prefix(&format!("a=fmtp:{pt} apt=")))
                .map(String::from)
        };
        let m: Vec<&str> = lines[i].split(' ').collect();
        let mut kept: Vec<String> = Vec::new();
        for want in keep {
            for pt in &m[3..] {
                if name_of(pt).is_some_and(|n| n.eq_ignore_ascii_case(want)) {
                    kept.push(pt.to_string());
                    for rtx in &m[3..] {
                        if apt_of(rtx).as_deref() == Some(*pt) {
                            kept.push(rtx.to_string());
                        }
                    }
                }
            }
        }
        out.push(format!("{} {}", m[..3].join(" "), kept.join(" ")));
        for l in sec {
            let pt = ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"]
                .iter()
                .find_map(|p| l.strip_prefix(p))
                .map(|r| r.split([' ', ':']).next().unwrap().to_string());
            if pt.is_none_or(|pt| kept.contains(&pt)) {
                out.push(l.to_string());
            }
        }
        i = end;
    }
    out.join("\r\n") + "\r\n"
}

fn counter(t: &RemoteTrack) -> Arc<Mutex<(u32, u32)>> {
    let c: Arc<Mutex<(u32, u32)>> = Arc::default();
    let c2 = c.clone();
    t.on_frame(move |rgba, w, h| {
        let mut g = c2.lock().unwrap();
        g.0 += 1;
        if is_box(rgba, w, h, 64) {
            g.1 += 1;
        }
    });
    c
}

fn source(run: &Arc<AtomicBool>) -> LocalTrack {
    let (v, w) = LocalTrack::video_source(VideoSourceOptions {
        width: 320,
        height: 240,
        fps: 30,
    })
    .unwrap();
    moving_box(w, 320, 240, 64, run.clone());
    v
}

/// The box arrives at `c`: at least 30 frames in 2 s, 80% of them clean.
fn flows(pair: &Pair, c: &Arc<Mutex<(u32, u32)>>, what: &str) {
    *c.lock().unwrap() = (0, 0);
    pair.wait(2000);
    let (n, clean) = *c.lock().unwrap();
    assert!(
        n >= 30 && clean * 10 >= n * 8,
        "{what}: {n} frames in 2 s, {clean} with the box"
    );
}

/// A `media-vp8` desktop offers both codecs, VP8 first; with
/// `prefer_h264`, H.264 first. A `media` desktop offers H.264 alone either
/// way. The answerer's codec choice is what is sent.
fn offers_list_our_codecs_in_order() {
    for prefer in [false, true] {
        testing::prefer_h264(prefer);
        let run = Arc::new(AtomicBool::new(true));
        let v = source(&run);
        // Read when each peer's connection is built, on the worker: on
        // until the offer is made.
        let pair = Pair::new();
        pair.a.add_track(&v).unwrap();
        let offer = drive(pair.a.create_offer(), T).unwrap();
        testing::prefer_h264(false);
        let want: &[&str] = if !cfg!(feature = "media-vp8") {
            &["H264"]
        } else if prefer {
            &["H264", "VP8"]
        } else {
            &["VP8", "H264"]
        };
        assert_eq!(video_codecs(&offer.sdp), want, "prefer_h264({prefer})");
        drive(pair.b.set_remote_description(offer), T).unwrap();
        let answer = drive(pair.b.create_answer(), T).unwrap();
        assert_eq!(
            video_codecs(&answer.sdp)[0],
            want[0],
            "the answer keeps the offer's order"
        );
        drive(pair.a.set_remote_description(answer), T).unwrap();
        pair.until("the track", || !pair.remotes.borrow().is_empty());
        let c = counter(&pair.remotes.borrow()[0]);
        flows(&pair, &c, want[0]);
        assert_eq!(
            testing::sending_codecs(&v),
            [want[0]],
            "the sender encodes the answer's codec"
        );
        run.store(false, Ordering::Relaxed);
        pair.close();
        pair.wait(300);
    }
}

/// An offer that lists H.264 only (Safari's can), or H.264 first: a default
/// desktop answers with H.264 only / first, and sends and receives H.264.
// One element without `media-vp8`.
#[allow(clippy::single_element_loop)]
fn an_h264_offer_is_answered_in_h264_both_ways() {
    for keep in [
        &["H264"][..],
        #[cfg(feature = "media-vp8")]
        &["H264", "VP8"][..],
    ] {
        let run = Arc::new(AtomicBool::new(true));
        let (va, vb) = (source(&run), source(&run));
        let pair = Pair::new();
        let at_a: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
        let r = at_a.clone();
        pair.a.on_track(move |t| r.borrow_mut().push(t));
        let errors: Rc<RefCell<Vec<String>>> = Rc::default();
        let e = errors.clone();
        pair.b
            .on_error(move |err| e.borrow_mut().push(err.to_string()));
        pair.a.add_track(&va).unwrap();
        let offer = drive(pair.a.create_offer(), T).unwrap();
        let munged = only(&offer.sdp, keep);
        assert_eq!(video_codecs(&munged), keep);
        drive(pair.b.set_remote_description(Description::offer(munged)), T).unwrap();
        pair.b.add_track(&vb).unwrap();
        let answer = drive(pair.b.create_answer(), T).unwrap();
        let got = video_codecs(&answer.sdp);
        assert_eq!(got[0], "H264", "{keep:?}: the answer {got:?}");
        assert!(
            got.iter().all(|c| keep.contains(&c.as_str())),
            "{keep:?}: the answer lists a codec the offer did not: {got:?}"
        );
        drive(pair.a.set_remote_description(answer), T).unwrap();
        pair.until("A's track at B", || !pair.remotes.borrow().is_empty());
        pair.until("B's track at A", || !at_a.borrow().is_empty());
        let (cb, ca) = (
            counter(&pair.remotes.borrow()[0]),
            counter(&at_a.borrow()[0]),
        );
        flows(&pair, &cb, "A -> B");
        flows(&pair, &ca, "B (the answerer) -> A");
        assert_eq!(testing::sending_codecs(&va), ["H264"]);
        assert_eq!(testing::sending_codecs(&vb), ["H264"]);
        assert!(
            errors.borrow().is_empty(),
            "{keep:?}: {:?}",
            errors.borrow()
        );
        run.store(false, Ordering::Relaxed);
        pair.close();
        pair.wait(300);
    }
}

/// Without `media-vp8`: an offer listing VP8 first and H.264 after it (what
/// every browser offers) is answered with H.264 alone, and H.264 flows both
/// ways: desktop video with Chrome or Safari needs no libvpx.
#[cfg(not(feature = "media-vp8"))]
fn a_vp8_first_offer_is_answered_in_h264() {
    let run = Arc::new(AtomicBool::new(true));
    let (va, vb) = (source(&run), source(&run));
    let pair = Pair::new();
    let at_a: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let r = at_a.clone();
    pair.a.on_track(move |t| r.borrow_mut().push(t));
    pair.a.add_track(&va).unwrap();
    let offer = drive(pair.a.create_offer(), T).unwrap();
    assert_eq!(
        video_codecs(&offer.sdp),
        ["H264"],
        "a media build offers H.264"
    );
    let munged = with_vp8_first(&offer.sdp);
    assert_eq!(video_codecs(&munged), ["VP8", "H264"]);
    drive(pair.b.set_remote_description(Description::offer(munged)), T).unwrap();
    pair.b.add_track(&vb).unwrap();
    let answer = drive(pair.b.create_answer(), T).unwrap();
    assert_eq!(
        video_codecs(&answer.sdp),
        ["H264"],
        "the answer to a VP8-first offer"
    );
    drive(pair.a.set_remote_description(answer), T).unwrap();
    pair.until("A's track at B", || !pair.remotes.borrow().is_empty());
    pair.until("B's track at A", || !at_a.borrow().is_empty());
    let (cb, ca) = (
        counter(&pair.remotes.borrow()[0]),
        counter(&at_a.borrow()[0]),
    );
    flows(&pair, &cb, "A -> B");
    flows(&pair, &ca, "B (the answerer) -> A");
    assert_eq!(testing::sending_codecs(&va), ["H264"]);
    assert_eq!(testing::sending_codecs(&vb), ["H264"]);
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);
}

/// A VP8 call renegotiated to H.264 only (the far side re-offers without
/// VP8): the answer lists H.264 only, the sender switches, and the receiver
/// switches decoders: the picture keeps coming.
#[cfg(feature = "media-vp8")]
fn a_renegotiated_codec_change_keeps_the_picture() {
    let run = Arc::new(AtomicBool::new(true));
    let v = source(&run);
    let pair = Pair::new();
    pair.a.add_track(&v).unwrap();
    pair.connect(1);
    let c = counter(&pair.remotes.borrow()[0]);
    flows(&pair, &c, "VP8 before");
    assert_eq!(testing::sending_codecs(&v), ["VP8"]);
    let offer = drive(pair.b.create_offer(), T).unwrap();
    let munged = only(&offer.sdp, &["H264"]);
    drive(pair.a.set_remote_description(Description::offer(munged)), T).unwrap();
    let answer = drive(pair.a.create_answer(), T).unwrap();
    assert_eq!(
        video_codecs(&answer.sdp),
        ["H264"],
        "the answer to an H.264-only re-offer"
    );
    drive(pair.b.set_remote_description(answer), T).unwrap();
    pair.until("the sender in H.264", || {
        testing::sending_codecs(&v) == ["H264"]
    });
    pair.wait(1000);
    flows(&pair, &c, "H.264 after");
    run.store(false, Ordering::Relaxed);
    pair.close();
    pair.wait(300);
}

/// One test: the peers' runtime is one per process, on this thread.
#[test]
fn codec_negotiation() {
    init();
    offers_list_our_codecs_in_order();
    an_h264_offer_is_answered_in_h264_both_ways();
    #[cfg(not(feature = "media-vp8"))]
    a_vp8_first_offer_is_answered_in_h264();
    #[cfg(feature = "media-vp8")]
    a_renegotiated_codec_change_keeps_the_picture();
}
