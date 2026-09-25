//! E2E pin for issue #153: debug-channel commands must not be serialized
//! behind full paints.
//!
//! Drives pairs of back-to-back `click` commands through the raw rinch-debug
//! TCP protocol (length-prefixed JSON, see `rinch-debug/src/protocol.rs`)
//! against the `debug_click_latency` probe example at 900x600, then reads the
//! app-observed inter-click delta back out of the DOM via `get_text_content`.
//! Before the #153 fix the second click of a pair stalled behind a full
//! software paint (350ms-2s in a debug build at this size, depending on
//! content), so no pair could land inside a double-click threshold; with the
//! fix (pre-paint drain + post-paint self-wake + pipelined command batching)
//! every pair lands within a few milliseconds.
//!
//! Needs a display (X11/Wayland), so it is `#[ignore]`d for CI. Run manually:
//!
//! ```sh
//! cargo build -p rinch --example debug_click_latency --features debug
//! cargo test -p rinch --test debug_event_latency -- --ignored
//! ```
//!
//! The first command is not optional: the test does not build the probe
//! itself (#275). It looks for it beside its own binary, in the `examples/`
//! directory of the same target dir and profile, and fails naming the command
//! when it is not there — so build it again after changing rinch, or the test
//! measures the probe as it was last built.
//!
//! Headless: start `Xvfb :99 -screen 0 1280x720x24 &` and run both commands
//! with `DISPLAY=:99` (or wrap them in `xvfb-run -a`).
#![cfg(feature = "desktop")] // serde_json comes in via the desktop feature

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Write a length-prefixed frame (4-byte big-endian length + JSON payload).
fn write_frame(stream: &mut TcpStream, data: &[u8]) {
    let len = data.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .expect("write frame len");
    stream.write_all(data).expect("write frame body");
    stream.flush().expect("flush frame");
}

/// Read a length-prefixed frame (4-byte big-endian length + JSON payload).
fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).expect("read frame len");
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).expect("read frame body");
    buf
}

/// Send one request and wait for its response.
fn request(stream: &mut TcpStream, req: &serde_json::Value) -> serde_json::Value {
    write_frame(stream, req.to_string().as_bytes());
    serde_json::from_slice(&read_frame(stream)).expect("parse response")
}

/// Kill the probe app when the test exits (pass or panic).
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Locate the probe example that the cargo invocation running this test built.
///
/// Found from this test binary's own location, at run time: a test binary
/// lives in `<target>/<profile>/deps/`, and an example of the same build in
/// `<target>/<profile>/examples/`. Two things this deliberately does not do
/// any more (#275). It does not bake a path in with `env!("CARGO_MANIFEST_DIR")`
/// — a shared target dir can hand this test the binary a since-deleted git
/// worktree built, and a baked path keeps pointing into that worktree. And it
/// does not run `cargo build` itself: a cargo spawned from inside a test
/// contends for the target-dir lock the invoking cargo may hold, which under a
/// full-workspace run is a hang rather than an error. A missing probe is a
/// failure that says how to build it, never a skip.
fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let probe = profile_dir
        .join("examples")
        .join(format!("debug_click_latency{}", std::env::consts::EXE_SUFFIX));
    assert!(
        probe.is_file(),
        "probe example not found at {} — build it first with the same \
         profile and target dir as this test:\n  \
         cargo build -p rinch --example debug_click_latency --features debug",
        probe.display()
    );
    probe
}

#[test]
#[ignore = "needs a display (X11/Wayland or Xvfb); see module docs"]
fn back_to_back_debug_clicks_are_not_serialized_behind_paints() {
    let binary = probe_binary();
    println!("probe: {}", binary.display());

    // Reserve a free port for the probe's debug server.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        listener.local_addr().unwrap().port()
    };

    let child = Command::new(&binary)
        .env("RINCH_DEBUG", "1")
        .env("RINCH_DEBUG_PORT", port.to_string())
        .spawn()
        .expect("spawn probe app");
    let mut guard = ChildGuard(child);

    // Connect (the debug server starts before the event loop, so this is quick).
    let mut stream = {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => panic!("could not connect to probe debug server: {e}"),
            }
        }
    };
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

    // Handshake.
    write_frame(&mut stream, br#"{"protocol":"rinch-debug","version":1}"#);
    let handshake: serde_json::Value =
        serde_json::from_slice(&read_frame(&mut stream)).expect("parse handshake");
    assert_eq!(handshake["protocol"], "rinch-debug");

    // Wait for the DOM to exist (window creation + first paint in a debug
    // build can take a while), polling for the delta node.
    let mut next_id: u64 = 1;
    let delta_node_id = {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let resp = request(
                &mut stream,
                &serde_json::json!({
                    "id": next_id,
                    "method": "query_selector",
                    "params": { "selector": ".click-delta" },
                }),
            );
            next_id += 1;
            if let Some(id) = resp["data"].get(0).and_then(|n| n["id"].as_u64()) {
                break id;
            }
            assert!(
                Instant::now() < deadline,
                "probe app never exposed .click-delta: {resp}"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
    };
    // Let startup paints settle so the first pair starts from an idle loop.
    std::thread::sleep(Duration::from_secs(2));

    // Drive pairs of back-to-back clicks. Both request frames are written
    // before reading responses, so the server dispatches click N+1 the moment
    // it has answered click N — no client round trip in the critical path.
    // Click N's response is sent before its induced paint, so without the
    // fix click N+1 stalls a full paint; with the pipelined batching window
    // it is processed within a couple of milliseconds. Several pairs absorb
    // scheduling jitter; assert on the best pair.
    let mut deltas = Vec::new();
    for _ in 0..5 {
        let click = |id: u64| {
            serde_json::json!({
                "id": id,
                "method": "click",
                "params": { "x": 450.0, "y": 300.0 },
            })
        };
        write_frame(&mut stream, click(next_id).to_string().as_bytes());
        write_frame(&mut stream, click(next_id + 1).to_string().as_bytes());
        next_id += 2;
        let _resp1: serde_json::Value =
            serde_json::from_slice(&read_frame(&mut stream)).expect("parse click 1");
        let _resp2: serde_json::Value =
            serde_json::from_slice(&read_frame(&mut stream)).expect("parse click 2");

        // The handler recorded the intra-pair delta while processing click 2,
        // before its response was sent — read it back out of the DOM.
        let resp = request(
            &mut stream,
            &serde_json::json!({
                "id": next_id,
                "method": "get_text_content",
                "params": { "id": delta_node_id },
            }),
        );
        next_id += 1;
        let text = resp["data"]
            .as_str()
            .expect("delta text")
            .trim()
            .to_string();
        let delta: f64 = text
            .parse()
            .unwrap_or_else(|_| panic!("unexpected .click-delta content: {text:?}"));
        deltas.push(delta);

        // Let the pair's paints finish so the next pair starts from idle.
        std::thread::sleep(Duration::from_millis(800));
    }

    let best = deltas.iter().cloned().fold(f64::INFINITY, f64::min);
    println!("intra-pair click deltas (ms): {deltas:?}; best: {best:.1}");

    // Shut the probe down gracefully and verify the drained exit behaves
    // (winit's exit is deferred; the app must still terminate cleanly). The
    // guard kills the process if it doesn't.
    write_frame(
        &mut stream,
        serde_json::json!({ "id": next_id, "method": "close_app" })
            .to_string()
            .as_bytes(),
    );
    let exited = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match guard.0.try_wait().expect("poll probe app") {
                Some(_) => break true,
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                None => break false,
            }
        }
    };
    assert!(exited, "probe app did not exit after close_app");

    assert!(
        best < 100.0,
        "no click pair landed inside 100ms (deltas: {deltas:?}); \
         debug commands are being serialized behind paints (#153)"
    );
}
