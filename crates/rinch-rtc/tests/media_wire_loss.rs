//! Packet loss on the wire (#1947's third review, adopted): a UDP relay
//! between the peers drops 3% of video RTP, retransmissions included, so
//! NACK and retransmission can repair it, as they could not when the
//! sender's track was never bound to the interceptors (every loss froze
//! the picture for about 600 ms: 1% loss was 30% of the time frozen).
//! `media_loss` drops packets after the interceptors and so tests the
//! keyframe path instead. Synthetic sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::RefCell;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::media::moving_box;
use common::{T, drive, init, pump_until};
use rinch_rtc::media::{LocalTrack, RemoteTrack, VideoSourceOptions};
use rinch_rtc::{IceCandidate, Peer, PeerState, RtcConfig, testing};

/// NACKs (RTCP transport feedback, FMT 1) seen on the wire, both ways.
static NACKS: AtomicU64 = AtomicU64::new(0);

struct Relay {
    listen: SocketAddr,
    dropped: Arc<AtomicU64>,
}

/// A relay for `target`: packets to `listen` go to `target` from another
/// socket; replies go back to whoever last sent to `listen`. Video RTP (PT
/// 96 VP8, 102 H.264) is dropped with probability `p_ppm`/1e6, retransmissions
/// included.
fn relay(target: SocketAddr, p_ppm: u64, run: Arc<AtomicBool>, seed: u64) -> Relay {
    let l = UdpSocket::bind("127.0.0.1:0").unwrap();
    let o = UdpSocket::bind("127.0.0.1:0").unwrap();
    l.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    o.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    let listen = l.local_addr().unwrap();
    let client: Arc<Mutex<Option<SocketAddr>>> = Arc::default();
    let dropped = Arc::new(AtomicU64::new(0));
    let rng = Arc::new(Mutex::new(seed));
    let lossy = {
        let (dropped, rng) = (dropped.clone(), rng.clone());
        move |b: &[u8]| -> bool {
            if b.len() > 8 && b[1] == 205 && b[0] & 0x1f == 1 {
                NACKS.fetch_add(1, Ordering::Relaxed);
            }
            if b.len() > 12 && (128..192).contains(&b[0]) {
                let pt = b[1] & 0x7f;
                if pt == 96 || pt == 102 {
                    let mut r = rng.lock().unwrap();
                    *r = r
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    if (*r >> 33) % 1_000_000 < p_ppm {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        return true;
                    }
                }
            }
            false
        }
    };
    {
        let (l, o, client, run, lossy) = (
            l.try_clone().unwrap(),
            o.try_clone().unwrap(),
            client.clone(),
            run.clone(),
            lossy.clone(),
        );
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while run.load(Ordering::Relaxed) {
                if let Ok((n, from)) = l.recv_from(&mut buf) {
                    *client.lock().unwrap() = Some(from);
                    if !lossy(&buf[..n]) {
                        let _ = o.send_to(&buf[..n], target);
                    }
                }
            }
        });
    }
    {
        let (l, o, client, run) = (l, o, client, run);
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while run.load(Ordering::Relaxed) {
                if let Ok((n, _)) = o.recv_from(&mut buf) {
                    let c = *client.lock().unwrap();
                    if let Some(c) = c
                        && !lossy(&buf[..n])
                    {
                        let _ = l.send_to(&buf[..n], c);
                    }
                }
            }
        });
    }
    Relay { listen, dropped }
}

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;

fn trickle(peer: &Peer) -> Inbox {
    let inbox: Inbox = Rc::default();
    let i = inbox.clone();
    peer.on_ice_candidate(move |c| i.borrow_mut().push(c));
    inbox
}

fn addr_of(c: &IceCandidate) -> Option<SocketAddr> {
    let f: Vec<&str> = c.candidate.split_whitespace().collect();
    format!("{}:{}", f.get(4)?, f.get(5)?).parse().ok()
}

fn rewrite(c: &IceCandidate, to: SocketAddr) -> IceCandidate {
    let mut f: Vec<String> = c.candidate.split_whitespace().map(String::from).collect();
    f[4] = to.ip().to_string();
    f[5] = to.port().to_string();
    IceCandidate {
        candidate: f.join(" "),
        sdp_mid: c.sdp_mid.clone(),
        sdp_m_line_index: c.sdp_m_line_index,
    }
}

fn call(codec: &str, p_ppm: u64, secs: u64) {
    let (w, h) = (320u32, 240u32);
    let run = Arc::new(AtomicBool::new(true));
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: w,
        height: h,
        fps: 30,
    })
    .unwrap();
    moving_box(vw, w, h, 64, run.clone());
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let (a_out, b_out) = (trickle(&a), trickle(&b));
    let remotes: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    let r = remotes.clone();
    b.on_track(move |t| r.borrow_mut().push(t));
    a.add_track(&video).unwrap();
    let relays: RefCell<Vec<Relay>> = RefCell::default();
    let mut seed = 7u64;
    let mut pump = || {
        for c in a_out.borrow_mut().drain(..) {
            match c.as_ref().and_then(|c| addr_of(c).map(|ad| (c, ad))) {
                Some((c, ad)) => {
                    seed += 1;
                    let rl = relay(ad, p_ppm, run.clone(), seed);
                    b.add_ice_candidate(Some(rewrite(c, rl.listen)));
                    relays.borrow_mut().push(rl);
                }
                None => b.add_ice_candidate(c),
            }
        }
        for c in b_out.borrow_mut().drain(..) {
            match c.as_ref().and_then(|c| addr_of(c).map(|ad| (c, ad))) {
                Some((c, ad)) => {
                    seed += 1;
                    let rl = relay(ad, p_ppm, run.clone(), seed);
                    a.add_ice_candidate(Some(rewrite(c, rl.listen)));
                    relays.borrow_mut().push(rl);
                }
                None => a.add_ice_candidate(c),
            }
        }
    };
    let offer = drive(a.create_offer(), T).unwrap();
    drive(b.set_remote_description(offer), T).unwrap();
    let answer = drive(b.create_answer(), T).unwrap();
    drive(a.set_remote_description(answer), T).unwrap();
    assert!(pump_until(
        || {
            pump();
            a.state() == PeerState::Connected
                && b.state() == PeerState::Connected
                && !remotes.borrow().is_empty()
        },
        Duration::from_secs(15)
    ));
    assert_eq!(testing::sending_codecs(&video), [codec]);
    let frames: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let f = frames.clone();
    remotes.borrow()[0].on_frame(move |_, _, _| f.lock().unwrap().push(Instant::now()));
    let _ = pump_until(
        || {
            pump();
            false
        },
        Duration::from_millis(2000),
    );
    let t0 = Instant::now();
    let k0 = testing::keyframe_requests(&video);
    let d0: u64 = relays
        .borrow()
        .iter()
        .map(|r| r.dropped.load(Ordering::Relaxed))
        .sum();
    NACKS.store(0, Ordering::Relaxed);
    frames.lock().unwrap().clear();
    let _ = pump_until(
        || {
            pump();
            false
        },
        Duration::from_secs(secs),
    );
    let shown = frames.lock().unwrap().clone();
    let dropped: u64 = relays
        .borrow()
        .iter()
        .map(|r| r.dropped.load(Ordering::Relaxed))
        .sum::<u64>()
        - d0;
    let nacks = NACKS.load(Ordering::Relaxed);
    // Time without a new picture, counting only gaps over 200 ms.
    let mut frozen = 0f64;
    let mut prev = t0;
    for t in shown
        .iter()
        .copied()
        .chain(std::iter::once(t0 + Duration::from_secs(secs)))
    {
        let g = t.saturating_duration_since(prev).as_secs_f64();
        if g > 0.2 {
            frozen += g;
        }
        prev = t;
    }
    let keyframes = testing::keyframe_requests(&video) - k0;
    eprintln!(
        "{codec} at {:.1}% loss: {dropped} packets dropped, {nacks} NACKs, {keyframes} keyframe requests, {} frames, frozen {:.0}% of {secs} s",
        p_ppm as f64 / 1e4,
        shown.len(),
        100.0 * frozen / secs as f64
    );
    assert!(
        dropped > 0,
        "{codec}: the relay dropped nothing (no loss tested)"
    );
    assert!(
        frozen / (secs as f64) < 0.05,
        "{codec}: frozen {frozen:.1} s of {secs} s at {:.1}% loss ({dropped} dropped, {nacks} NACKs)",
        p_ppm as f64 / 1e4
    );
    // One NACK a loss, give or take a lost retransmission: not a NACK
    // every 100 ms until the packet ages out.
    assert!(
        nacks <= dropped * 3,
        "{codec}: {nacks} NACKs for {dropped} losses"
    );
    run.store(false, Ordering::Relaxed);
    a.close();
    b.close();
    let _ = pump_until(|| false, Duration::from_millis(300));
}

#[test]
// One element without `media-vp8`.
#[allow(clippy::single_element_loop)]
fn wire_loss_is_repaired_by_retransmission() {
    init();
    for codec in [
        #[cfg(feature = "media-vp8")]
        "VP8",
        "H264",
    ] {
        testing::h264_only(codec == "H264");
        call(codec, 30_000, 8);
    }
    testing::h264_only(false);
}
