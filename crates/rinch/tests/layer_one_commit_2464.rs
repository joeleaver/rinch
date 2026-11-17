//! Issue #2464 on a live compositor: what an app asks of a Wayland layer
//! surface in one turn of the event loop reaches the compositor in one
//! commit, in whatever order it asked.
//!
//! The probe (`examples/layer_commit_probe.rs`) makes two calls one after
//! another in one `run_on_main_thread` closure and is run with
//! `WAYLAND_DEBUG=client`, which prints every request it sends. Between the
//! probe's markers the test counts the `wl_surface.commit`s that carry
//! layer-surface state (`zwlr_layer_surface_v1.set_size`, `set_anchor`,
//! `set_margin`, `set_exclusive_zone`, … sent since the commit before):
//! exactly one, in both orders. The frame that follows, a new buffer at the
//! new size, is a commit too but carries no layer state, and an
//! `ack_configure` is the reply to the compositor's configure, not a change.
//!
//! Needs a compositor with `wlr-layer-shell`, so every test is `#[ignore]`d.
//! Run it inside an isolated session (`scripts/isolated-session.sh`, issue
//! #2429) on the session's own headless KWin, never on your desktop:
//!
//! ```sh
//! cargo build -p rinch --example layer_commit_probe
//! cargo test -p rinch --test layer_one_commit_2464 --no-run
//! scripts/isolated-session.sh --kwin -- \
//!   cargo test -p rinch --test layer_one_commit_2464 -- --ignored --test-threads=1
//! ```
#![cfg(all(feature = "desktop", target_os = "linux"))]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// What a layer surface's state is made of: the requests that change it.
const LAYER_STATE: &[&str] = &[
    "set_size",
    "set_anchor",
    "set_margin",
    "set_exclusive_zone",
    "set_keyboard_interactivity",
    "set_layer",
    "set_exclusive_edge",
];

fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let probe = profile_dir.join("examples").join("layer_commit_probe");
    assert!(
        probe.is_file(),
        "probe not found at {} — build it first:\n  \
         cargo build -p rinch --example layer_commit_probe",
        probe.display()
    );
    probe
}

/// The requests (`interface.request`, in order) the probe sent between its
/// markers for `command`.
fn requests_during(overlay: bool, command: &str) -> Vec<String> {
    assert!(
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        "needs a Wayland compositor: run inside scripts/isolated-session.sh --kwin"
    );
    let mut cmd = Command::new(probe_binary());
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("RINCH_RENDERER", "software")
        .env("WAYLAND_DEBUG", "client")
        .env("RINCH_DEBUG", "0");
    if overlay {
        cmd.env("PROBE_OVERLAY", "1");
    }
    let mut child = cmd.spawn().expect("spawn the probe");
    let mut stdin = child.stdin.take().unwrap();
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = |what: &str| {
        rx.recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| panic!("the probe printed nothing more while waiting for {what}"))
    };
    // Mapped: its first frame went out (a commit after the first configure).
    let mut configured = false;
    loop {
        let line = next("its first frame");
        match request_of(&line).as_deref() {
            Some("zwlr_layer_surface_v1.ack_configure") => configured = true,
            Some("wl_surface.commit") if configured => break,
            _ => {}
        }
    }
    // Let any startup traffic settle before the change.
    std::thread::sleep(Duration::from_millis(500));
    while rx.try_recv().is_ok() {}
    writeln!(stdin, "{command}").unwrap();
    stdin.flush().unwrap();
    let begin = format!("PROBE-BEGIN {command}");
    let end = format!("PROBE-END {command}");
    while next(&begin) != begin {}
    let mut requests = Vec::new();
    loop {
        let line = next(&end);
        if line == end {
            break;
        }
        if let Some(request) = request_of(&line) {
            requests.push(request);
        }
    }
    let _ = writeln!(stdin, "quit");
    let _ = child.wait();
    requests
}

/// `interface.request` from a `WAYLAND_DEBUG` request line, `None` for
/// anything else (an event). libwayland writes an object as `wl_surface#7`
/// (`[1158677.414]  -> wl_surface#7.commit()`), wayland-rs's own backend as
/// `wl_surface@7`.
fn request_of(line: &str) -> Option<String> {
    let rest = line.split(" -> ").nth(1)?;
    let (interface, rest) = rest.split_once(['#', '@'])?;
    let request = rest.split_once('.')?.1.split_once('(')?.0;
    Some(format!("{}.{request}", interface.trim()))
}

/// How many commits carried layer-surface state, and which state requests
/// were sent.
fn layer_commits(requests: &[String]) -> (usize, Vec<String>) {
    let mut commits = 0;
    let mut pending = false;
    let mut sent = Vec::new();
    for r in requests {
        if let Some(name) = r.strip_prefix("zwlr_layer_surface_v1.") {
            if LAYER_STATE.contains(&name) {
                pending = true;
                sent.push(name.to_string());
            }
        } else if r == "wl_surface.commit" && pending {
            commits += 1;
            pending = false;
        }
    }
    assert!(
        !pending,
        "layer state sent and never committed: {requests:?}"
    );
    (commits, sent)
}

fn assert_one_commit(overlay: bool, command: &str, wanted: &[&str]) {
    let requests = requests_during(overlay, command);
    let (commits, sent) = layer_commits(&requests);
    for w in wanted {
        assert!(
            sent.iter().any(|s| s == w),
            "{command}: no {w} in {requests:?}"
        );
    }
    assert_eq!(
        commits, 1,
        "{command}: layer state went out in {commits} commits: {requests:?}"
    );
}

#[test]
fn parser_reads_wayland_debug_lines() {
    assert_eq!(
        request_of("[1158677.414]  -> wl_surface#31.commit()").as_deref(),
        Some("wl_surface.commit")
    );
    assert_eq!(
        request_of("[ 1234.567] -> wl_surface@7.commit()").as_deref(),
        Some("wl_surface.commit")
    );
    assert_eq!(
        request_of("[ 1234.567] -> zwlr_layer_surface_v1@12.set_size(800, 60)").as_deref(),
        Some("zwlr_layer_surface_v1.set_size")
    );
    assert_eq!(
        request_of("[ 1234.567] zwlr_layer_surface_v1@12.configure(1, 800, 60)"),
        None
    );
    let r = |s: &str| s.to_string();
    let two = [
        r("zwlr_layer_surface_v1.set_anchor"),
        r("wl_surface.commit"),
        r("zwlr_layer_surface_v1.set_size"),
        r("wl_surface.attach"),
        r("wl_surface.commit"),
    ];
    assert_eq!(layer_commits(&two).0, 2);
    let one = [
        r("zwlr_layer_surface_v1.set_size"),
        r("zwlr_layer_surface_v1.set_anchor"),
        r("wl_surface.commit"),
        r("zwlr_layer_surface_v1.ack_configure"),
        r("wl_surface.attach"),
        r("wl_surface.commit"),
    ];
    assert_eq!(layer_commits(&one).0, 1);
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_panel_resized_then_moved_is_one_commit() {
    assert_one_commit(
        false,
        "size-then-layer",
        &["set_size", "set_anchor", "set_exclusive_zone"],
    );
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_panel_moved_then_resized_is_one_commit() {
    assert_one_commit(
        false,
        "layer-then-size",
        &["set_size", "set_anchor", "set_exclusive_zone"],
    );
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn an_overlay_resized_then_positioned_is_one_commit() {
    assert_one_commit(true, "size-then-position", &["set_size", "set_margin"]);
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn an_overlay_positioned_then_resized_is_one_commit() {
    assert_one_commit(true, "position-then-size", &["set_size", "set_margin"]);
}
