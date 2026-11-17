//! Issue #2501 on a live compositor: the token a window gets from
//! `request_activation_token` in an Enter handler lets another program take
//! the keyboard; a token that program makes for itself does not.
//!
//! The probe (`examples/activation_token_probe.rs`) asks for a token when
//! Enter is pressed in it. The test presses Enter through the compositor
//! (`keys.rs`), as a keyboard does, so the press has a serial the token can
//! name.
//!
//! - **An already-running program** (`waiter.rs`, a plain toplevel mapped
//!   before the probe) is given the probe's token and asks to be activated
//!   with it: it gets the keyboard. Asking with a token it made itself (for
//!   its own unfocused surface, with no input serial) it does not.
//! - **A program started with the token** (`foot --app-id child`, which
//!   activates its first window with `XDG_ACTIVATION_TOKEN`) gets the
//!   keyboard. This checks the token reaches a child and is used. It cannot
//!   show a difference: both compositors give a *new* window the focus when
//!   it maps, token or not (KWin 6 at every focus-stealing-prevention level
//!   from 2 to 4, sway unless a `no_focus` rule forbids it, which then holds
//!   against the token too, since sway reads an activation before the map as
//!   startup notification only). The already-running program is where a
//!   token decides.
//!
//! sway is set to `focus_on_window_activation focus` (its default marks the
//! window urgent instead); KWin runs as it comes.
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
mod waiter;

use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// How long something that should happen gets: long, since a loaded machine
/// can be slow, and a test that waits for it returns as soon as it happens.
const PATIENCE: Duration = Duration::from_secs(30);
/// How long a test watches for something that must not happen.
const QUIET: Duration = Duration::from_secs(3);

fn session_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RINCH_SESSION_DIR")
            .expect("run inside scripts/isolated-session.sh (RINCH_SESSION_DIR is not set)"),
    )
}

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
}

/// The compositor's Wayland socket name: the session's KWin, or a headless
/// sway this test process starts once (the session ends it).
fn display() -> &'static str {
    static DISPLAY: OnceLock<String> = OnceLock::new();
    DISPLAY.get_or_init(|| {
        if std::env::var_os("ACTIVATION_TEST_SWAY").is_some() {
            start_sway()
        } else {
            std::env::var("WAYLAND_DISPLAY").expect(
                "needs a compositor: scripts/isolated-session.sh --kwin, or ACTIVATION_TEST_SWAY=1",
            )
        }
    })
}

/// A headless sway in the session that focuses a window asking to be
/// activated with a token it trusts; answers its socket's name. It runs for
/// the rest of the test process, held here, and the session ends it.
fn start_sway() -> String {
    static SWAY: OnceLock<Mutex<Child>> = OnceLock::new();
    let runtime = runtime_dir();
    let config = session_dir().join("sway.conf");
    std::fs::write(&config, "focus_on_window_activation focus\n")
        .expect("write sway's configuration");
    let before = sockets(&runtime);
    let sway = Command::new("sway")
        .arg("--config")
        .arg(&config)
        .env("WLR_BACKENDS", "headless")
        .env("WLR_RENDERER", "pixman")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env_remove("WAYLAND_DISPLAY")
        .arg("--debug")
        .stdout(Stdio::null())
        // Its debug log, for a failure's reader.
        .stderr(std::fs::File::create(session_dir().join("sway.log")).expect("sway's log"))
        .spawn()
        .expect("start sway");
    assert!(SWAY.set(Mutex::new(sway)).is_ok(), "sway started once");
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(name) = sockets(&runtime).into_iter().find(|s| !before.contains(s)) {
            return name;
        }
        assert!(Instant::now() < deadline, "sway made no Wayland socket");
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
}

impl Probe {
    /// Starts the probe with `env`, and waits until its window has the
    /// keyboard.
    fn start(env: &[(&str, &Path)]) -> Self {
        let mut command = Command::new(probe_binary());
        command
            .env("WAYLAND_DISPLAY", display())
            .env("RINCH_RENDERER", "software")
            .env("RINCH_DEBUG", "0")
            .env_remove("XDG_ACTIVATION_TOKEN")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("start the probe");
        let stderr = BufReader::new(child.stderr.take().expect("the probe's stderr"));
        let (tx, lines) = mpsc::channel();
        // Everything it says goes to the session's `probe.log` too, for a
        // failure's reader (run with `RINCH_SESSION_KEEP=1`, and
        // `WAYLAND_DEBUG=client` for its protocol log).
        let mut log = std::fs::File::create(session_dir().join("probe.log")).ok();
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
        let probe = Self { child, lines };
        probe.wait_for("PROBE ready");
        probe
    }

    /// Waits for the line `expected`, failing on a line that says something
    /// went wrong.
    fn wait_for(&self, expected: &str) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line == expected => return,
                Ok(line) if line.starts_with("PROBE token") || line.starts_with("PROBE not") => {
                    panic!("waiting for {expected:?}, the probe said {line:?}")
                }
                Ok(_) => {}
                Err(_) => panic!("the probe never said {expected:?}"),
            }
        }
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
        .map(|dir| dir.join("examples").join("activation_token_probe"))
        .find(|probe| probe.is_file());
    found.unwrap_or_else(|| {
        panic!(
            "probe not found in {candidates:?} — build it first:\n  \
             cargo build -p rinch --example activation_token_probe"
        )
    })
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

/// The waiter mapped and then left by the keyboard for the probe, which
/// has asked for a token from an Enter press and written it to `file`; and
/// the keyboard, which the test keeps: on a headless sway it is the seat's
/// only one, and without it no window is given the keyboard at all.
fn waiter_behind_the_probe(file: &Path) -> (waiter::Waiter, Probe, keys::Keyboard) {
    let runtime = runtime_dir();
    let mut keyboard = keys::Keyboard::new(&socket(), &runtime);
    let waiter = waiter::Waiter::start(&socket(), &runtime);
    assert!(wait_until(PATIENCE, || waiter.mapped() && waiter.focused()));
    let _ = std::fs::remove_file(file);
    let probe = Probe::start(&[("PROBE_TOKEN_FILE", file)]);
    assert!(
        wait_until(PATIENCE, || !waiter.focused()),
        "the probe's new window took the keyboard"
    );
    keyboard.press_enter();
    probe.wait_for("PROBE token ok");
    (waiter, probe, keyboard)
}

#[test]
#[ignore = "needs a compositor: run inside scripts/isolated-session.sh"]
fn a_running_program_given_the_token_gets_the_keyboard() {
    let file = session_dir().join("token");
    let (waiter, _probe, _keyboard) = waiter_behind_the_probe(&file);
    let token = std::fs::read_to_string(&file).expect("the probe's token");
    waiter.activate_with(&token);
    assert!(
        wait_until(PATIENCE, || waiter.focused()),
        "activated with the probe's token, the waiter did not get the keyboard"
    );
}

#[test]
#[ignore = "needs a compositor: run inside scripts/isolated-session.sh"]
fn a_running_program_with_a_token_of_its_own_does_not() {
    let file = session_dir().join("token");
    let (waiter, _probe, _keyboard) = waiter_behind_the_probe(&file);
    waiter.activate_with_own_token();
    assert!(
        !wait_until(QUIET, || waiter.focused()),
        "activated with a token of its own, the waiter took the keyboard"
    );
}

#[test]
#[ignore = "needs foot and a compositor: run inside scripts/isolated-session.sh"]
fn a_program_started_with_the_token_gets_the_keyboard() {
    let runtime = runtime_dir();
    let mut keyboard = keys::Keyboard::new(&socket(), &runtime);
    let log = session_dir().join("child.log");
    let child = "env WAYLAND_DEBUG=1 foot --app-id child sleep 8";
    let probe = Probe::start(&[("PROBE_CHILD", Path::new(child)), ("PROBE_CHILD_LOG", &log)]);
    keyboard.press_enter();
    probe.wait_for("PROBE token ok");
    probe.wait_for("PROBE started");
    let used_and_focused = wait_until(PATIENCE, || {
        std::fs::read_to_string(&log).is_ok_and(|text| {
            let activated = text.contains("xdg_activation_v1") && text.contains(".activate(");
            let entered = text
                .lines()
                .any(|line| line.contains("wl_keyboard#") && line.contains(".enter("));
            activated && entered
        })
    });
    assert!(
        used_and_focused,
        "foot did not activate its window with the token and get the keyboard: {}",
        log.display()
    );
}
