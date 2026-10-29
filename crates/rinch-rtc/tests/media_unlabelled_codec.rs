//! A video sender whose codec label is not the one negotiated (#1954).
//!
//! webrtc-rs 0.21 binds a sender to its interceptors (the NACK responder,
//! the sender reports) once, when negotiation completes, and only if the
//! codec its track is labelled with is among the negotiated ones. A track
//! is labelled with the codec negotiation will most likely pick, which a
//! peer cannot always know when the track is added:
//!
//! - the answerer added its track before the offer arrived, and the offer
//!   leaves the label out (an H.264-only offer to a `media-vp8` build);
//! - the offerer's own label (its first codec) is left out of the answer.
//!
//! Such a sender sent video but no sender report and no retransmission.
//! Both shapes here, in turn: a UDP relay between the peers counts the
//! sender reports carrying the video sender's SSRC. Without `media-vp8`
//! H.264 is the only codec, so the label is always negotiated and both
//! shapes simply pass. Synthetic sources only.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::RefCell;
use std::collections::HashSet;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::media::moving_box;
use common::{T, drive, init, pump_until};
use rinch_rtc::media::{Direction, LocalTrack, MediaKind, RemoteTrack, VideoSourceOptions};
use rinch_rtc::{Description, IceCandidate, Peer, PeerState, RtcConfig, testing};

/// H.264's payload type in rinch-rtc's SDP.
const H264_PT: u8 = 102;

/// What the relays saw: the SSRCs of H.264 RTP, and sender reports by SSRC.
#[derive(Default)]
struct Wire {
    video_ssrcs: HashSet<u32>,
    reports: Vec<u32>,
}

fn wire() -> &'static Mutex<Wire> {
    static W: std::sync::OnceLock<Mutex<Wire>> = std::sync::OnceLock::new();
    W.get_or_init(Mutex::default)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Note one datagram. SRTP and SRTCP leave the headers read here in the
/// clear: the RTP SSRC (bytes 8..12) and the first RTCP packet's type and
/// sender SSRC (bytes 1 and 4..8).
fn note(b: &[u8]) {
    if b.len() < 12 || !(128..192).contains(&b[0]) {
        return;
    }
    let mut w = wire().lock().unwrap();
    if b[1] == 200 {
        w.reports.push(be32(&b[4..8]));
    } else if !(192..=223).contains(&b[1]) && b[1] & 0x7f == H264_PT {
        w.video_ssrcs.insert(be32(&b[8..12]));
    }
}

/// Sender reports from a video sender so far.
fn video_reports() -> usize {
    let w = wire().lock().unwrap();
    w.reports
        .iter()
        .filter(|s| w.video_ssrcs.contains(s))
        .count()
}

/// A relay for `target`: packets to the returned address go to `target`
/// from another socket; replies go back to whoever last sent. Everything
/// passes; `note` sees it.
fn relay(target: SocketAddr, run: Arc<AtomicBool>) -> SocketAddr {
    let l = UdpSocket::bind("127.0.0.1:0").unwrap();
    let o = UdpSocket::bind("127.0.0.1:0").unwrap();
    l.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    o.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
    let listen = l.local_addr().unwrap();
    let client: Arc<Mutex<Option<SocketAddr>>> = Arc::default();
    {
        let (l, o, client, run) = (
            l.try_clone().unwrap(),
            o.try_clone().unwrap(),
            client.clone(),
            run.clone(),
        );
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while run.load(Ordering::Relaxed) {
                if let Ok((n, from)) = l.recv_from(&mut buf) {
                    *client.lock().unwrap() = Some(from);
                    note(&buf[..n]);
                    let _ = o.send_to(&buf[..n], target);
                }
            }
        });
    }
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while run.load(Ordering::Relaxed) {
            if let Ok((n, _)) = o.recv_from(&mut buf) {
                let c = *client.lock().unwrap();
                if let Some(c) = c {
                    note(&buf[..n]);
                    let _ = l.send_to(&buf[..n], c);
                }
            }
        }
    });
    listen
}

type Inbox = Rc<RefCell<Vec<Option<IceCandidate>>>>;

fn trickle(peer: &Peer) -> Inbox {
    let inbox: Inbox = Rc::default();
    let i = inbox.clone();
    peer.on_ice_candidate(move |c| i.borrow_mut().push(c));
    inbox
}

/// A candidate with its address replaced by a relay's toward it.
fn relayed(c: &IceCandidate, run: &Arc<AtomicBool>) -> IceCandidate {
    let mut f: Vec<String> = c.candidate.split_whitespace().map(String::from).collect();
    let target: SocketAddr = format!("{}:{}", f[4], f[5]).parse().unwrap();
    let to = relay(target, run.clone());
    f[4] = to.ip().to_string();
    f[5] = to.port().to_string();
    IceCandidate {
        candidate: f.join(" "),
        sdp_mid: c.sdp_mid.clone(),
        sdp_m_line_index: c.sdp_m_line_index,
    }
}

/// `sdp` with VP8 taken out of every video m-line (the m-line's payload
/// list and the payload's attributes). Nothing changes without `media-vp8`.
fn drop_vp8(sdp: &str) -> String {
    let vp8: Vec<String> = sdp
        .split("\r\n")
        .filter_map(|l| l.strip_prefix("a=rtpmap:"))
        .filter_map(|r| r.split_once(' '))
        .filter(|(_, codec)| codec.to_ascii_uppercase().starts_with("VP8/"))
        .map(|(pt, _)| pt.to_string())
        .collect();
    let mut out = Vec::new();
    for l in sdp.split("\r\n").filter(|l| !l.is_empty()) {
        if l.starts_with("m=video") {
            let kept: Vec<&str> = l
                .split(' ')
                .filter(|p| !vp8.iter().any(|v| v == p))
                .collect();
            out.push(kept.join(" "));
            continue;
        }
        let attr_pt = ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"]
            .iter()
            .find_map(|p| l.strip_prefix(p))
            .and_then(|r| r.split(' ').next());
        if attr_pt.is_some_and(|pt| vp8.iter().any(|v| v == pt)) {
            continue;
        }
        out.push(l.to_string());
    }
    out.join("\r\n") + "\r\n"
}

/// The two shapes in turn (the peers share process-wide state).
#[test]
fn a_sender_whose_label_negotiation_left_out_still_sends_reports() {
    init();
    for answerer_sends in [true, false] {
        call(answerer_sends);
    }
}

/// One call, video one way over relays, in H.264 though the sender's
/// track was labelled with this build's first codec. `answerer_sends`:
/// the answerer added its track before an offer without VP8; otherwise
/// the offerer sends and the answer leaves VP8 out.
fn call(answerer_sends: bool) {
    let shape = if answerer_sends {
        "track added before an H.264-only offer"
    } else {
        "offerer's label left out of the answer"
    };
    *wire().lock().unwrap() = Wire::default();
    let run = Arc::new(AtomicBool::new(true));
    let (video, vw) = LocalTrack::video_source(VideoSourceOptions {
        width: 320,
        height: 240,
        fps: 30,
    })
    .unwrap();
    moving_box(vw, 320, 240, 64, run.clone());
    let a = Peer::new(RtcConfig::default()).unwrap();
    let b = Peer::new(RtcConfig::default()).unwrap();
    let (a_out, b_out) = (trickle(&a), trickle(&b));
    let remotes: Rc<RefCell<Vec<RemoteTrack>>> = Rc::default();
    for p in [&a, &b] {
        let r = remotes.clone();
        p.on_track(move |t| r.borrow_mut().push(t));
    }
    // The sender's track before any description, so it is labelled with
    // this build's first codec.
    if answerer_sends {
        b.add_track(&video).unwrap();
        // The offer's video m-line. A transceiver with no track: it sends
        // nothing (native `RecvOnly` is refused by webrtc-rs, #1992).
        a.add_transceiver(MediaKind::Video, Direction::SendRecv)
            .unwrap();
    } else {
        a.add_track(&video).unwrap();
    }
    let pump = || {
        for c in a_out.borrow_mut().drain(..) {
            b.add_ice_candidate(c.map(|c| relayed(&c, &run)));
        }
        for c in b_out.borrow_mut().drain(..) {
            a.add_ice_candidate(c.map(|c| relayed(&c, &run)));
        }
    };
    let mut offer = drive(a.create_offer(), T).unwrap();
    if answerer_sends {
        offer = Description::offer(drop_vp8(&offer.sdp));
    }
    drive(b.set_remote_description(offer), T).unwrap();
    let mut answer = drive(b.create_answer(), T).unwrap();
    if !answerer_sends {
        answer = Description::answer(drop_vp8(&answer.sdp));
    }
    assert!(!answer.sdp.contains(" VP8/"), "{shape}: {}", answer.sdp);
    drive(a.set_remote_description(answer), T).unwrap();
    assert!(
        pump_until(
            || {
                pump();
                a.state() == PeerState::Connected
                    && b.state() == PeerState::Connected
                    && !remotes.borrow().is_empty()
            },
            Duration::from_secs(15)
        ),
        "{shape}: connected, the track arrived"
    );
    let frames = Arc::new(AtomicU64::new(0));
    let f = frames.clone();
    remotes.borrow()[0].on_frame(move |_, _, _| {
        f.fetch_add(1, Ordering::Relaxed);
    });
    // Sender reports go out about once a second.
    let _ = pump_until(
        || {
            pump();
            false
        },
        Duration::from_millis(3500),
    );
    assert_eq!(testing::sending_codecs(&video), ["H264"], "{shape}");
    let shown = frames.load(Ordering::Relaxed);
    let ssrcs = wire().lock().unwrap().video_ssrcs.len();
    let reports = video_reports();
    eprintln!("{shape}: {shown} frames shown, {ssrcs} video SSRC, {reports} sender reports");
    // A positive control: video crossed the relays.
    assert!(shown > 30, "{shape}: video flows ({shown} frames)");
    assert_eq!(ssrcs, 1, "{shape}: one video sender on the wire");
    assert!(
        reports >= 2,
        "{shape}: the video sender sends reports ({reports} in 3.5 s), so it is bound to the interceptors"
    );
    run.store(false, Ordering::Relaxed);
    a.close();
    b.close();
    let _ = pump_until(|| false, Duration::from_millis(300));
}
