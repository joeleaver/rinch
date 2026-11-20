//! Issue #2731 on a live compositor: a key typed the moment a window has the
//! keyboard reaches the field the app focused, and the field is focused by
//! the time the app is told the window has the keyboard.
//!
//! The probe (`examples/focus_race_probe.rs`) reports what it saw on stderr.
//! The test types through the compositor's virtual keyboard (`keys.rs`), as
//! a keyboard does, so the keys arrive with the compositor's own timing
//! relative to the keyboard enter.
//!
//! - **A window shown at start** whose field is focused while the component
//!   is built: the test types from before the window maps until after it has
//!   the keyboard. Every key the window was given must land in the field,
//!   and when `window_focused()` turns true the field must already be the
//!   active element (labwc showed it was not, for about 20 ms), its `focus`
//!   event already fired.
//! - **A shortcut that opens the field**: `o` opens a hidden overlay and moves
//!   the focus into it (`focus_into`, as `Modal` does), and `hello` follows
//!   in the same burst. Before #2731 the keys that arrived in the turn of the
//!   `o` were lost: the overlay's focus waited for the turn's end.
//!
//! Needs sway or labwc, so every test is `#[ignore]`d. Run them inside an
//! isolated session (`scripts/isolated-session.sh`, issue #2429), never on
//! your desktop:
//!
//! ```sh
//! cargo build -p rinch --example focus_race_probe
//! cargo test -p rinch --test focus_race_2731 --no-run
//! FOCUS_RACE_COMPOSITOR=sway scripts/isolated-session.sh -- \
//!   cargo test -p rinch --test focus_race_2731 -- --ignored --test-threads=1
//! FOCUS_RACE_COMPOSITOR=labwc scripts/isolated-session.sh -- \
//!   cargo test -p rinch --test focus_race_2731 -- --ignored --test-threads=1
//! ```
#![cfg(all(feature = "windowing", target_os = "linux"))]

mod keys;

use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// How long something that should happen gets: long, since a loaded machine
/// can be slow, and a test that waits for it returns as soon as it happens.
const PATIENCE: Duration = Duration::from_secs(30);
/// How long the probe is left to finish handling keys already sent.
const SETTLE: Duration = Duration::from_millis(1500);

fn session_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RINCH_SESSION_DIR")
            .expect("run inside scripts/isolated-session.sh (RINCH_SESSION_DIR is not set)"),
    )
}

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
}

/// The compositor's Wayland socket name: a headless sway or labwc this test
/// process starts once (the session ends it), as `FOCUS_RACE_COMPOSITOR`
/// says.
fn display() -> &'static str {
    static DISPLAY: OnceLock<String> = OnceLock::new();
    DISPLAY.get_or_init(|| {
        let which = std::env::var("FOCUS_RACE_COMPOSITOR").unwrap_or_else(|_| "sway".into());
        start_compositor(&which)
    })
}

fn start_compositor(which: &str) -> String {
    static COMPOSITOR: OnceLock<Mutex<Child>> = OnceLock::new();
    let runtime = runtime_dir();
    let session = session_dir();
    let before = sockets(&runtime);
    let mut command = match which {
        "sway" => {
            let config = session.join("sway.conf");
            std::fs::write(&config, "").expect("write sway's configuration");
            let mut c = Command::new("sway");
            c.arg("--config").arg(&config).arg("--debug");
            c
        }
        "labwc" => {
            let config = session.join("labwc");
            std::fs::create_dir_all(&config).expect("labwc's configuration directory");
            let mut c = Command::new("labwc");
            c.arg("-C").arg(&config).arg("-d");
            c
        }
        other => panic!("FOCUS_RACE_COMPOSITOR={other}: sway or labwc"),
    };
    let child = command
        .env("WLR_BACKENDS", "headless")
        .env("WLR_RENDERER", "pixman")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env_remove("WAYLAND_DISPLAY")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(session.join(format!("{which}.log"))).expect("its log"))
        .spawn()
        .unwrap_or_else(|e| panic!("start {which}: {e}"));
    assert!(
        COMPOSITOR.set(Mutex::new(child)).is_ok(),
        "compositor started once"
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(name) = sockets(&runtime).into_iter().find(|s| !before.contains(s)) {
            return name;
        }
        assert!(Instant::now() < deadline, "{which} made no Wayland socket");
        std::thread::sleep(Duration::from_millis(50));
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

fn socket() -> PathBuf {
    runtime_dir().join(display())
}

/// The probe, running, with its `PROBE` lines; killed on drop.
struct Probe {
    child: Child,
    lines: mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl Probe {
    fn start(mode: &str) -> Self {
        let mut command = Command::new(probe_binary());
        command
            .env("WAYLAND_DISPLAY", display())
            .env("RINCH_RENDERER", "software")
            .env("RINCH_DEBUG", "0")
            .env("PROBE_MODE", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("start the probe");
        let stderr = BufReader::new(child.stderr.take().expect("the probe's stderr"));
        let (tx, lines) = mpsc::channel();
        let mut log = std::fs::File::create(session_dir().join(format!("probe-{mode}.log"))).ok();
        std::thread::spawn(move || {
            for line in stderr.lines().map_while(Result::ok) {
                if let Some(log) = &mut log {
                    let _ = writeln!(log, "{line}");
                }
                if line.starts_with("PROBE") && tx.send(line.clone()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            seen: Vec::new(),
        }
    }

    /// Takes every line the probe has said so far.
    fn poll(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
    }

    fn said(&self, line: &str) -> bool {
        self.seen.iter().any(|l| l == line)
    }

    fn wait_for(&mut self, line: &str) {
        let deadline = Instant::now() + PATIENCE;
        while !self.said(line) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => self.seen.push(l),
                Err(_) => panic!("the probe never said {line:?}: {:?}", self.seen),
            }
        }
    }

    fn settle(&mut self) {
        std::thread::sleep(SETTLE);
        self.poll();
    }

    /// The keys the window was given, and the field's last value.
    fn keys_and_value(&self) -> (Vec<&str>, &str) {
        let keys = self
            .seen
            .iter()
            .filter_map(|l| l.strip_prefix("PROBE key "))
            .collect();
        let value = self
            .seen
            .iter()
            .rev()
            .find_map(|l| l.strip_prefix("PROBE value "))
            .unwrap_or("");
        (keys, value)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The probe: beside this test binary (`<target>/<profile>/examples/`), or,
/// when Cargo's `build-dir` keeps test binaries apart from the final
/// artifacts, in the workspace's `target/<profile>/examples/`.
fn probe_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("FOCUS_RACE_PROBE") {
        return PathBuf::from(path);
    }
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
        .map(|dir| dir.join("examples").join("focus_race_probe"))
        .find(|probe| probe.is_file());
    found.unwrap_or_else(|| {
        panic!(
            "probe not found in {candidates:?} — build it first:\n  \
             cargo build -p rinch --example focus_race_probe"
        )
    })
}

/// A window shown at start, its field focused while built: typing from
/// before the window maps until well after it has the keyboard, every key
/// the window is given lands in the field, and when the app hears the window
/// has the keyboard the field is already focused, its `focus` event fired.
#[test]
#[ignore = "needs sway or labwc: run inside scripts/isolated-session.sh"]
fn every_key_reaches_a_field_focused_at_start() {
    let mut keyboard = keys::Keyboard::new(&socket(), &runtime_dir());
    let mut probe = Probe::start("start");
    let deadline = Instant::now() + PATIENCE;
    let mut ready_at = None;
    loop {
        keyboard.type_burst("h");
        std::thread::sleep(Duration::from_millis(2));
        probe.poll();
        if ready_at.is_none() && probe.said("PROBE ready") {
            ready_at = Some(Instant::now());
        }
        if ready_at.is_some_and(|t| t.elapsed() > Duration::from_millis(300)) {
            break;
        }
        assert!(Instant::now() < deadline, "never ready: {:?}", probe.seen);
    }
    probe.settle();
    let focused = probe
        .seen
        .iter()
        .find(|l| l.starts_with("PROBE window-focused"))
        .cloned();
    assert_eq!(
        focused.as_deref(),
        Some("PROBE window-focused active=field"),
        "when the window got the keyboard: {:?}",
        probe.seen
    );
    let onfocus = probe.seen.iter().position(|l| l == "PROBE field-onfocus");
    let window = probe
        .seen
        .iter()
        .position(|l| l.starts_with("PROBE window-focused"));
    assert!(
        onfocus.is_some() && onfocus < window,
        "the field's focus event fires before the app hears the window is focused: {:?}",
        probe.seen
    );
    let (keys, value) = probe.keys_and_value();
    assert!(
        !keys.is_empty(),
        "positive control: the window was given keys"
    );
    assert_eq!(
        value.len(),
        keys.len(),
        "{} keys reached the window and {} the field: {:?}",
        keys.len(),
        value.len(),
        probe.seen
    );
}

/// `o` opens an overlay and moves the focus into its field; `hello`, in the
/// same burst, lands in it.
#[test]
#[ignore = "needs sway or labwc: run inside scripts/isolated-session.sh"]
fn keys_right_after_a_shortcut_opens_a_field_reach_it() {
    let mut keyboard = keys::Keyboard::new(&socket(), &runtime_dir());
    let mut probe = Probe::start("shortcut");
    probe.wait_for("PROBE ready");
    keyboard.type_burst("ohello");
    probe.settle();
    let (keys, value) = probe.keys_and_value();
    assert_eq!(
        keys,
        ["o", "h", "e", "l", "l", "o"],
        "positive control: the window was given every key: {:?}",
        probe.seen
    );
    assert_eq!(value, "hello", "{:?}", probe.seen);
}
