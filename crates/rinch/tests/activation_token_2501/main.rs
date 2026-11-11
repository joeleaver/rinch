//! Issue #2501 on a live compositor: a window that starts a program from an
//! Enter press with an xdg-activation token gives that program's window the
//! keyboard; without a token it does not.
//!
//! The probe (`examples/activation_token_probe.rs`) starts
//! `foot --app-id child` (foot activates its window with
//! `XDG_ACTIVATION_TOKEN`) when Enter is pressed in it; the test presses Enter
//! through the compositor (`keys.rs`), so the press has a serial, and reads
//! the child's `WAYLAND_DEBUG` log for `wl_keyboard.enter`.
//!
//! Each compositor is set up to tell the two apart, since by default both
//! focus a new window whatever it brings:
//!
//! - **sway** (started by the test, headless, inside the session):
//!   `no_focus [app_id="child"]` keeps the child from taking the focus when
//!   it maps, and `focus_on_window_activation focus` lets its activation with
//!   the token take it.
//! - **KWin** (the session's, `--kwin`): focus-stealing prevention at
//!   "extreme", so only an activation the compositor can trust gets the
//!   focus.
//!
//! Needs foot and a compositor, so every test is `#[ignore]`d. Run them inside
//! an isolated session (`scripts/isolated-session.sh`, issue #2429), never on
//! your desktop:
//!
//! ```sh
//! cargo build -p rinch --example activation_token_probe
//! cargo test -p rinch --test activation_token_2501 --no-run
//! # KWin: its fake input is a restricted interface.
//! KWIN_WAYLAND_NO_PERMISSION_CHECKS=1 scripts/isolated-session.sh --kwin -- \
//!   cargo test -p rinch --test activation_token_2501 -- --ignored --test-threads=1
//! # sway, started by the test.
//! ACTIVATION_TEST_SWAY=1 scripts/isolated-session.sh -- \
//!   cargo test -p rinch --test activation_token_2501 -- --ignored --test-threads=1
//! ```
#![cfg(all(feature = "desktop", target_os = "linux"))]

mod keys;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The compositor the test runs on, and how its clients reach it.
struct Compositor {
    /// The Wayland socket's name, in `runtime`.
    display: String,
    runtime: PathBuf,
    /// sway, when the test started it.
    sway: Option<Child>,
}

impl Drop for Compositor {
    fn drop(&mut self) {
        if let Some(sway) = &mut self.sway {
            let _ = sway.kill();
            let _ = sway.wait();
        }
    }
}

fn session_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RINCH_SESSION_DIR")
            .expect("run inside scripts/isolated-session.sh (RINCH_SESSION_DIR is not set)"),
    )
}

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
}

/// The session's compositor, set up to tell a trusted activation from a new
/// window that only maps.
fn compositor() -> Compositor {
    let runtime = runtime_dir();
    if std::env::var_os("ACTIVATION_TEST_SWAY").is_some() {
        return sway(runtime);
    }
    let display = std::env::var("WAYLAND_DISPLAY").expect(
        "needs a compositor: scripts/isolated-session.sh --kwin, or ACTIVATION_TEST_SWAY=1",
    );
    kwin_extreme_focus_stealing_prevention();
    Compositor {
        display,
        runtime,
        sway: None,
    }
}

/// A headless sway in the session, with the child kept from taking the focus
/// on its own.
fn sway(runtime: PathBuf) -> Compositor {
    let config = session_dir().join("sway.conf");
    std::fs::write(
        &config,
        "no_focus [app_id=\"child\"]\nfocus_on_window_activation focus\n",
    )
    .expect("write sway's configuration");
    let before = sockets(&runtime);
    let child = Command::new("sway")
        .arg("--config")
        .arg(&config)
        .env("WLR_BACKENDS", "headless")
        .env("WLR_RENDERER", "pixman")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start sway");
    let deadline = Instant::now() + Duration::from_secs(10);
    let display = loop {
        if let Some(name) = sockets(&runtime).into_iter().find(|s| !before.contains(s)) {
            break name;
        }
        assert!(Instant::now() < deadline, "sway made no Wayland socket");
        std::thread::sleep(Duration::from_millis(50));
    };
    Compositor {
        display,
        runtime,
        sway: Some(child),
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

/// KWin's focus-stealing prevention at its strictest, so a window gets the
/// focus only from an activation KWin trusts.
fn kwin_extreme_focus_stealing_prevention() {
    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("XDG_CONFIG_HOME"));
    std::fs::write(
        config.join("kwinrc"),
        "[Windows]\nFocusStealingPreventionLevel=4\n",
    )
    .expect("write kwinrc");
    let reconfigured = Command::new("dbus-send")
        .args([
            "--session",
            "--type=method_call",
            "--dest=org.kde.KWin",
            "/KWin",
            "org.kde.KWin.reconfigure",
        ])
        .status()
        .expect("run dbus-send");
    assert!(reconfigured.success(), "KWin did not reconfigure");
}

/// Whether the child got the keyboard: starts the probe, presses Enter in
/// it, and reads the child's log.
fn child_gets_the_keyboard(with_token: bool) -> bool {
    let compositor = compositor();
    let socket = compositor.runtime.join(&compositor.display);
    let mut keyboard = keys::Keyboard::new(&socket, &compositor.runtime);
    let log = session_dir().join(format!("child-{with_token}.log"));
    let _ = std::fs::remove_file(&log);
    let mut probe = Command::new(probe_binary())
        .env("WAYLAND_DISPLAY", &compositor.display)
        .env("RINCH_RENDERER", "software")
        .env("RINCH_DEBUG", "0")
        .env(
            "PROBE_CHILD",
            "env WAYLAND_DEBUG=1 foot --app-id child sleep 8",
        )
        .env("PROBE_CHILD_LOG", &log)
        .env("PROBE_TOKEN", if with_token { "1" } else { "0" })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the probe");
    let lines = probe_lines(&mut probe);
    wait_for(&lines, "PROBE ready");
    // The probe's window has the keyboard once it is mapped and focused.
    std::thread::sleep(Duration::from_millis(500));
    keyboard.press_enter();
    if with_token {
        wait_for(&lines, "PROBE token ok");
    }
    wait_for(&lines, "PROBE started");
    let got_it = wait_until(Duration::from_secs(5), || {
        std::fs::read_to_string(&log).is_ok_and(|text| keyboard_entered(&text))
    });
    let mapped = std::fs::read_to_string(&log).is_ok_and(|text| text.contains("xdg_toplevel"));
    let _ = probe.kill();
    let _ = probe.wait();
    assert!(mapped, "the child never made its window: {}", log.display());
    got_it
}

/// Whether a client's `WAYLAND_DEBUG` log shows its keyboard entering one of
/// its surfaces.
fn keyboard_entered(log: &str) -> bool {
    log.lines()
        .any(|line| line.contains("wl_keyboard#") && line.contains(".enter("))
}

fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let probe = profile_dir.join("examples").join("activation_token_probe");
    assert!(
        probe.is_file(),
        "probe not found at {} — build it first:\n  \
         cargo build -p rinch --example activation_token_probe",
        probe.display()
    );
    probe
}

/// The probe's `PROBE` lines, as they come.
fn probe_lines(probe: &mut Child) -> mpsc::Receiver<String> {
    let stderr = BufReader::new(probe.stderr.take().expect("the probe's stderr"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            if line.starts_with("PROBE") && tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Waits for the probe line `expected`, failing on any other `PROBE` line
/// that says something went wrong.
fn wait_for(lines: &mpsc::Receiver<String>, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) if line == expected => return,
            Ok(line) if line.starts_with("PROBE token") || line.starts_with("PROBE not") => {
                panic!("waiting for {expected:?}, the probe said {line:?}")
            }
            Ok(_) => {}
            Err(_) => panic!("the probe never said {expected:?}"),
        }
    }
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

#[test]
#[ignore = "needs foot and a compositor: run inside scripts/isolated-session.sh"]
fn with_a_token_the_started_program_gets_the_keyboard() {
    assert!(child_gets_the_keyboard(true));
}

#[test]
#[ignore = "needs foot and a compositor: run inside scripts/isolated-session.sh"]
fn without_a_token_the_started_program_does_not() {
    assert!(!child_gets_the_keyboard(false));
}
