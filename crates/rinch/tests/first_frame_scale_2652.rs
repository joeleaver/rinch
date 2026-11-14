//! Issue #2652 on a live compositor: on a scaled output, a window's first
//! frame is painted at the output's scale, and no frame is painted at
//! another.
//!
//! A compositor tells a surface its scale only once the surface is mapped,
//! which takes a first buffer: headless sway sends `preferred_scale` right
//! after the first `attach`. rinch used to paint that buffer at winit's
//! default scale 1 and then everything again at the output's. It now creates
//! the surface at the scale it will most likely get (the winit fork's
//! `WindowAttributesWayland::with_initial_scale_factor`).
//!
//! The probe (`examples/first_frame_scale_probe.rs`, 240x120 logical) runs
//! with `RINCH_PERF=1` and the software renderer, and every painted frame's
//! `surface_px` (its buffer's area) is read from its `[PERF] frame` lines.
//! sway floats every window, so no tiling resize makes a frame of its own.
//!
//! Needs sway, so every test is `#[ignore]`d. Run them inside an isolated
//! session (`scripts/isolated-session.sh`, issue #2429), never on your
//! desktop; the test starts one headless sway per test:
//!
//! ```sh
//! cargo build -p rinch --features desktop --example first_frame_scale_probe
//! cargo test -p rinch --features desktop --test first_frame_scale_2652 --no-run
//! scripts/isolated-session.sh -- \
//!   cargo test -p rinch --features desktop --test first_frame_scale_2652 -- --ignored
//! ```
#![cfg(all(feature = "desktop", target_os = "linux"))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long sway gets to make its socket.
const PATIENCE: Duration = Duration::from_secs(30);

/// The probe's logical size.
const SIZE: (f64, f64) = (240.0, 120.0);

fn session_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RINCH_SESSION_DIR")
            .expect("run inside scripts/isolated-session.sh (RINCH_SESSION_DIR is not set)"),
    )
}

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
}

/// A headless sway with one 1920x1080 output at `scale`, killed on drop.
struct Sway {
    child: Child,
    display: String,
}

impl Sway {
    fn start(name: &str, scale: f64) -> Self {
        let runtime = runtime_dir();
        let config = session_dir().join(format!("sway-{name}.conf"));
        std::fs::write(
            &config,
            format!(
                "output HEADLESS-1 resolution 1920x1080 scale {scale}\n\
                 for_window [title=\".*\"] floating enable\n"
            ),
        )
        .expect("write sway's configuration");
        let started = std::time::SystemTime::now();
        let child = Command::new("sway")
            .arg("--config")
            .arg(&config)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::null())
            .stderr(
                std::fs::File::create(session_dir().join(format!("sway-{name}.log")))
                    .expect("sway's log"),
            )
            .spawn()
            .expect("start sway");
        let deadline = Instant::now() + PATIENCE;
        let display = loop {
            // A killed sway's socket can be left behind, and the next one
            // may take its name: the new socket is the one made since.
            let fresh = sockets(&runtime).into_iter().find(|name| {
                std::fs::metadata(runtime.join(name))
                    .and_then(|m| m.modified())
                    .is_ok_and(|made| made >= started)
            });
            if let Some(name) = fresh {
                break name;
            }
            assert!(Instant::now() < deadline, "sway made no Wayland socket");
            std::thread::sleep(Duration::from_millis(50));
        };
        Self { child, display }
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        // SIGTERM, so sway removes its socket; then it is waited for.
        let _ = Command::new("kill")
            .arg(self.child.id().to_string())
            .status();
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `wayland-N` sockets in `dir`.
fn sockets(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("wayland-") && !name.ends_with(".lock"))
        .collect()
}

fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let profile = profile_dir.file_name().expect("a profile directory");
    let workspace_target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let candidates = [profile_dir.to_path_buf(), workspace_target.join(profile)];
    let found = candidates
        .iter()
        .map(|dir| dir.join("examples").join("first_frame_scale_probe"))
        .find(|probe| probe.is_file());
    found.unwrap_or_else(|| {
        panic!(
            "probe not found in {candidates:?} — build it first:\n  \
             cargo build -p rinch --features desktop --example first_frame_scale_probe"
        )
    })
}

/// What a probe run showed: the `surface_px` of each frame it painted, and
/// the document's scale after each show, in order.
struct Run {
    frames: Vec<u64>,
    scales: Vec<f64>,
}

/// Runs the probe on a fresh sway at `scale` with `env`; `{SWAYSOCK}` in an
/// env value is that sway's IPC socket.
fn run_probe(name: &str, scale: f64, env: &[(&str, &str)]) -> Run {
    let sway = Sway::start(name, scale);
    let ipc = runtime_dir().join(format!(
        "sway-ipc.{}.{}.sock",
        // The uid sway names its socket with: the runtime directory's owner.
        std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(runtime_dir()).unwrap()),
        sway.child.id()
    ));
    let mut command = Command::new(probe_binary());
    command
        .env("WAYLAND_DISPLAY", &sway.display)
        .env("RINCH_RENDERER", "software")
        .env("RINCH_PERF", "1")
        .env("RINCH_DEBUG", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value.replace("{SWAYSOCK}", &ipc.to_string_lossy()));
    }
    let output = command.output().expect("run the probe");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Kept for a failure's reader (`RINCH_SESSION_KEEP=1`).
    let _ = std::fs::write(session_dir().join(format!("probe-{name}.log")), &*stderr);
    assert!(output.status.success(), "the probe failed:\n{stderr}");
    let frames: Vec<u64> = stderr
        .lines()
        .filter(|line| line.starts_with("[PERF] frame "))
        .filter_map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("surface_px="))
                .map(|px| px.parse().expect("surface_px is a number"))
        })
        .collect();
    assert!(!frames.is_empty(), "the probe painted nothing:\n{stderr}");
    let scales = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("PROBE scale "))
        .map(|s| s.trim().parse().expect("a scale"))
        .collect();
    Run { frames, scales }
}

/// [`run_probe`]'s painted frames, checking that the document's scale after
/// each show is `scale`.
fn painted_frames(name: &str, scale: f64, env: &[(&str, &str)]) -> Vec<u64> {
    let run = run_probe(name, scale, env);
    assert!(
        !run.scales.is_empty() && run.scales.iter().all(|&s| s == scale),
        "the document's scale should be {scale}: {:?}",
        run.scales
    );
    run.frames
}

/// The buffer area of the probe's window at `scale`.
fn area_at(scale: f64) -> u64 {
    ((SIZE.0 * scale).round() * (SIZE.1 * scale).round()) as u64
}

fn assert_all_at(frames: &[u64], scale: f64) {
    assert!(
        frames.iter().all(|&px| px == area_at(scale)),
        "every painted frame should be {} px (scale {scale}), not scale 1's {}: {frames:?}",
        area_at(scale),
        area_at(1.0),
    );
}

#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_toplevel_is_painted_at_the_outputs_scale_from_its_first_frame() {
    let frames = painted_frames("toplevel", 2.0, &[]);
    assert_all_at(&frames, 2.0);
    assert_eq!(frames.len(), 1, "one painted frame: {frames:?}");
}

#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_fractional_scale_is_painted_from_the_first_frame() {
    let frames = painted_frames("fractional", 1.5, &[]);
    assert_all_at(&frames, 1.5);
    assert_eq!(frames.len(), 1, "one painted frame: {frames:?}");
}

#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_hidden_start_is_painted_at_the_outputs_scale_when_shown() {
    let frames = painted_frames("hidden", 2.0, &[("PROBE_HIDDEN", "1")]);
    assert_all_at(&frames, 2.0);
    assert_eq!(frames.len(), 1, "one painted frame: {frames:?}");
}

/// Shown, hidden and shown again: each show's surface is new, and each is
/// painted at the output's scale.
#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_window_shown_again_is_painted_at_the_outputs_scale() {
    let frames = painted_frames(
        "reshown",
        2.0,
        &[("PROBE_HIDDEN", "1"), ("PROBE_SHOWS", "2")],
    );
    assert_all_at(&frames, 2.0);
    assert_eq!(frames.len(), 2, "one painted frame per show: {frames:?}");
}

/// sway gives a layer surface its output's scale before its first
/// configure, so this one held before #2652 too; it pins that the initial
/// scale does not disturb it.
#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_layer_surface_is_painted_at_its_outputs_scale() {
    let frames = painted_frames("layer", 2.0, &[("PROBE_LAYER", "1")]);
    assert_all_at(&frames, 2.0);
    assert_eq!(frames.len(), 1, "one painted frame: {frames:?}");
}

/// A window that asks for an output, shown, hidden while that output's
/// scale goes from 2 to 1, and shown again: the second show's surface starts
/// at the output's new scale, which is then no change the compositor
/// reports, and the document takes it from the new window.
#[test]
#[ignore = "needs sway: run inside scripts/isolated-session.sh"]
fn a_window_shown_after_its_output_was_rescaled_takes_the_new_scale() {
    let run = run_probe(
        "rescaled",
        2.0,
        &[
            ("PROBE_HIDDEN", "1"),
            ("PROBE_SHOWS", "2"),
            ("PROBE_OUTPUT", "HEADLESS-1"),
            (
                "PROBE_WHILE_HIDDEN",
                "swaymsg -s {SWAYSOCK} output HEADLESS-1 scale 1 && sleep 1",
            ),
        ],
    );
    assert_eq!(
        run.frames,
        [area_at(2.0), area_at(1.0)],
        "one painted frame per show"
    );
    assert_eq!(
        run.scales,
        [2.0, 1.0],
        "the document's scale after each show"
    );
}
