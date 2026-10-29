//! A process that exits with a peer still open closes it on the way out
//! (the worker's `atexit` handler), so the far end sees the association end
//! at once instead of when its ICE checks time out.
//!
//! The test re-runs its own binary as a child: the child connects to a lite
//! peer in this (parent) process, signalling over its stdin/stdout, and then
//! exits without closing anything.
//!
//! The child waits for the parent's `LEAVE` line before it leaves (#1972).
//! Every mode closes the peer as soon as the child leaves (`close()`, the
//! drop, the `atexit` close and `shutdown()` alike), so without that hand-off the
//! parent's "nothing closed yet" check raced the very close it waits for, and
//! failed whenever the parent was descheduled between reading `OPEN` and
//! asking the lite peer.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{LitePeer, T, drive, init, pump_until};
use rinch_rtc::{ChannelOptions, ChannelState, Description, Peer, Reliability, RtcConfig};

const CHILD: &str = "RINCH_RTC_EXIT_CHILD";

/// The child: connect, report OPEN, wait for LEAVE, exit with the peer open.
#[test]
fn exit_child() {
    if std::env::var(CHILD).is_err() {
        return;
    }
    init();
    let peer = Peer::new(RtcConfig::default()).unwrap();
    let control = peer
        .create_data_channel(
            "control",
            ChannelOptions::negotiated(0, Reliability::Reliable),
        )
        .unwrap();
    let offer = drive(peer.create_offer(), T).unwrap();
    println!("OFFER {}", offer.sdp.replace("\r\n", "\\n"));
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    let answer = line.trim_end().replace("\\n", "\r\n");
    drive(peer.set_remote_description(Description::answer(answer)), T).unwrap();
    assert!(pump_until(|| control.state() == ChannelState::Open, T));
    control.send(b"up".to_vec()).unwrap();
    let _ = pump_until(|| false, Duration::from_millis(300));
    println!("OPEN");
    std::io::stdout().flush().unwrap();
    // Leave only when the parent has checked that nothing closed yet.
    line.clear();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim_end(), "LEAVE", "the parent's go-ahead");
    // How the child leaves (review of #1909): with the peer still open,
    // right after close(), or right after dropping it. The last two used
    // to die before the in-flight close was sent.
    match std::env::var(CHILD).as_deref() {
        Ok("close") => peer.close(),
        Ok("drop") => drop(peer),
        // rinch_rtc::shutdown(), then an exit that runs no atexit handler
        // (what std::process::exit may be on Windows): shutdown alone must
        // have closed it.
        Ok("shutdown") => {
            rinch_rtc::shutdown();
            // SAFETY: ends the process at once, skipping atexit.
            unsafe { libc::_exit(0) }
        }
        _ => std::mem::forget(peer),
    }
    std::process::exit(0);
}

#[test]
fn the_far_end_sees_a_peer_close_when_its_process_exits() {
    if std::env::var(CHILD).is_ok() {
        return;
    }
    for how in ["open", "close", "drop", "shutdown"] {
        far_end_sees_the_close(how);
    }
}

fn far_end_sees_the_close(how: &str) {
    let lite = LitePeer::spawn();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "exit_child", "--nocapture", "--test-threads=1"])
        .env(CHILD, how)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut line = String::new();
    let deadline = Instant::now() + T;
    loop {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "child ended early");
        // libtest prints "test exit_child ... " ahead of the child's own
        // first line, on the same line.
        if let Some(at) = line.find("OFFER ") {
            let offer = &line.trim_end()[at + "OFFER ".len()..];
            let answer = lite.answer(&offer.replace("\\n", "\r\n"));
            writeln!(stdin, "{}", answer.replace("\r\n", "\\n")).unwrap();
        }
        if line.trim_end().ends_with("OPEN") {
            break;
        }
        assert!(Instant::now() < deadline, "child never opened");
    }
    assert_eq!(lite.stats().received.get("control"), Some(&2), "connected");
    // The child is parked on its stdin: nothing has closed the peer, and
    // nothing will until the LEAVE below.
    assert!(
        lite.stats().closed.is_empty(),
        "exit with the peer {how}: closed before the child was told to leave: {:?}",
        lite.stats()
    );
    writeln!(stdin, "LEAVE").unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "child exited {status:?}");
    // Well inside any ICE consent timeout: only a close sent at exit
    // explains a close seen this soon.
    let exited = Instant::now();
    while lite.stats().closed.is_empty() && exited.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !lite.stats().closed.is_empty(),
        "exit with the peer {how}: the lite peer saw no close within 1.5 s of \
         the client's exit: {:?}",
        lite.stats()
    );
}
