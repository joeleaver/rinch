//! Issue #1546 on a live X server: a window the user moved and resized,
//! hidden with `hide_current_window` and shown with `show_current_window`,
//! comes back at the same place and size — and a non-activating window is
//! still shown without taking the input focus.
//!
//! Needs an X server and `xdotool`/`xwininfo`/`xprop`, so every test is
//! `#[ignore]`d. Run it on a private server, never your desktop's:
//!
//! ```sh
//! Xwayland :97 -rootless &            # or Xvfb :97
//! cargo build -p rinch --example hide_show_geometry
//! DISPLAY=:97 cargo test -p rinch --test hide_show_geometry -- --ignored --test-threads=1 --skip wm_
//! ```
//!
//! The probe is found beside this test binary (see `debug_event_latency.rs`
//! for why it is not built from here). With no window manager the server
//! places a window exactly where it asks, so the geometry read back is the
//! geometry the shell requested.
#![cfg(all(feature = "desktop", target_os = "linux"))]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const TITLE: &str = "rinch-geom-probe";

fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let probe = profile_dir.join("examples").join("hide_show_geometry");
    assert!(
        probe.is_file(),
        "probe not found at {} — build it first:\n  \
         cargo build -p rinch --example hide_show_geometry",
        probe.display()
    );
    probe
}

struct Probe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Probe {
    fn spawn(envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(probe_binary());
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .env("RINCH_RENDERER", "software")
            // Logical = physical unless a test says otherwise, so the
            // expected numbers are the ones written.
            .env("WINIT_X11_SCALE_FACTOR", "1");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn the probe");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Probe {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// `current_window_position` and `current_window_size`, as the probe
    /// prints them.
    fn geom(&mut self) -> Vec<f64> {
        self.send("geom");
        let mut line = String::new();
        loop {
            line.clear();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "probe exited"
            );
            if let Some(rest) = line.trim().strip_prefix("geom ") {
                return rest.split(' ').map(|w| w.parse().unwrap()).collect();
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

fn run(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {cmd}: {e}"));
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// The probe's mapped window, waiting for it to appear.
fn visible_window() -> Option<String> {
    run("xdotool", &["search", "--onlyvisible", "--name", TITLE])
        .lines()
        .next()
        .map(str::to_string)
}

fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `(x, y, width, height)` of a window, from `xwininfo`.
fn geometry(wid: &str) -> (i32, i32, i32, i32) {
    let info = run("xwininfo", &["-id", wid]);
    let field = |name: &str| -> i32 {
        info.lines()
            .find_map(|l| l.trim().strip_prefix(name))
            .unwrap_or_else(|| panic!("no `{name}` in xwininfo:\n{info}"))
            .trim()
            .parse()
            .unwrap()
    };
    (
        field("Absolute upper-left X:"),
        field("Absolute upper-left Y:"),
        field("Width:"),
        field("Height:"),
    )
}

/// Wait until the window reports `want`.
fn settle(wid: &str, want: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = geometry(wid);
        if got == want || Instant::now() > deadline {
            return got;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Hide, wait until no window is mapped, show, and return the new window.
fn hide_and_show(probe: &mut Probe) -> String {
    probe.send("hide");
    wait_for("the window to go", || {
        visible_window().is_none().then_some(())
    });
    probe.send("show");
    wait_for("the window to come back", visible_window)
}

fn moved_and_resized_comes_back(envs: &[(&str, &str)]) -> (Probe, String) {
    moved_and_resized_comes_back_at(envs, (220, 224, 560, 280), (300, 378, 700, 200))
}

/// Start (expecting `start`, physical), move and resize to `to` (physical)
/// from outside, hide, show, and expect `to` again. Returns the probe (alive)
/// and the re-shown window.
fn moved_and_resized_comes_back_at(
    envs: &[(&str, &str)],
    start: (i32, i32, i32, i32),
    to: (i32, i32, i32, i32),
) -> (Probe, String) {
    let mut probe = Probe::spawn(envs);
    let wid = wait_for("the probe's window", visible_window);
    // Positive control: the window starts where WindowProps put it.
    assert_eq!(settle(&wid, start), start);

    // The user moves and resizes it.
    let (x, y, w, h) = to;
    run(
        "xdotool",
        &["windowmove", &wid, &x.to_string(), &y.to_string()],
    );
    run(
        "xdotool",
        &["windowsize", &wid, &w.to_string(), &h.to_string()],
    );
    assert_eq!(
        settle(&wid, to),
        to,
        "positive control: the move and resize reached the window"
    );
    std::thread::sleep(Duration::from_millis(300));

    let shown = hide_and_show(&mut probe);
    assert_eq!(
        settle(&shown, to),
        to,
        "the re-shown window is back where it was hidden, at its size"
    );
    (probe, shown)
}

#[test]
#[ignore = "needs an X server and xdotool; see module docs"]
fn a_moved_and_resized_window_comes_back_at_its_geometry() {
    moved_and_resized_comes_back(&[]);
}

/// At a fractional scale the position comes back in desktop pixels and the
/// size through logical pixels, both to the pixel (701 / 1.5 is not whole).
#[test]
#[ignore = "needs an X server and xdotool; see module docs"]
fn at_a_fractional_scale_it_comes_back_to_the_pixel() {
    moved_and_resized_comes_back_at(
        &[("WINIT_X11_SCALE_FACTOR", "1.5")],
        (330, 336, 840, 420),
        (301, 377, 701, 201),
    );
}

#[test]
#[ignore = "needs an X server and xdotool; see module docs"]
fn a_non_activating_window_comes_back_at_its_geometry_without_the_focus() {
    let (_probe, shown) = moved_and_resized_comes_back(&[("PROBE_NON_ACTIVATING", "1")]);
    let hints = run("xprop", &["-id", &shown, "WM_HINTS"]);
    assert!(
        hints.contains("Client accepts input or input focus: False"),
        "the re-shown window still refuses the input focus: {hints}"
    );
    let state = run("xprop", &["-id", &shown, "_NET_WM_STATE"]);
    assert!(
        state.contains("_NET_WM_STATE_SKIP_TASKBAR"),
        "still off the taskbar: {state}"
    );
    let focus = run("xdotool", &["getwindowfocus"]);
    assert_ne!(
        focus.trim(),
        shown,
        "the re-shown window did not take focus"
    );
}

/// The getters and setters (#1546): they report the live window, keep the
/// geometry while it is hidden, and a geometry set while hidden is where the
/// window comes back.
#[test]
#[ignore = "needs an X server and xdotool; see module docs"]
fn the_geometry_api_reads_and_sets_the_window() {
    let mut probe = Probe::spawn(&[]);
    let wid = wait_for("the probe's window", visible_window);
    settle(&wid, (220, 224, 560, 280));
    assert_eq!(probe.geom(), [220.0, 224.0, 560.0, 280.0]);

    probe.send("move 260 300");
    probe.send("size 640 360");
    assert_eq!(settle(&wid, (260, 300, 640, 360)), (260, 300, 640, 360));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(probe.geom(), [260.0, 300.0, 640.0, 360.0]);

    probe.send("hide");
    wait_for("the window to go", || {
        visible_window().is_none().then_some(())
    });
    assert_eq!(
        probe.geom(),
        [260.0, 300.0, 640.0, 360.0],
        "kept while hidden"
    );
    probe.send("move 120 140");
    probe.send("size 500 250");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(probe.geom(), [120.0, 140.0, 500.0, 250.0]);
    probe.send("show");
    let shown = wait_for("the window to come back", visible_window);
    assert_eq!(settle(&shown, (120, 140, 500, 250)), (120, 140, 500, 250));
}

/// A hidden start (#1520) still appears at the requested position, and is
/// then restored like any other window.
#[test]
#[ignore = "needs an X server and xdotool; see module docs"]
fn a_hidden_start_appears_at_the_requested_position() {
    let mut probe = Probe::spawn(&[("PROBE_HIDDEN", "1")]);
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        visible_window().is_none(),
        "positive control: started hidden"
    );
    probe.send("show");
    let wid = wait_for("the first show", visible_window);
    assert_eq!(settle(&wid, (220, 224, 560, 280)), (220, 224, 560, 280));
    run("xdotool", &["windowmove", &wid, "333", "111"]);
    assert_eq!(settle(&wid, (333, 111, 560, 280)), (333, 111, 560, 280));
    std::thread::sleep(Duration::from_millis(300));
    let shown = hide_and_show(&mut probe);
    assert_eq!(settle(&shown, (333, 111, 560, 280)), (333, 111, 560, 280));
}

// ── Under a window manager ──────────────────────────────────────────────────
//
// A reparenting window manager puts a frame around the window, so the
// position a client asks for is the frame's and `xwininfo` reports the
// client's: these compare the window with itself before the hide rather than
// with fixed numbers. Run them inside a private headless KWin:
//
// ```sh
// dbus-run-session -- kwin_wayland --virtual --xwayland --socket wayland-test \
//     --no-lockscreen --no-global-shortcuts --exit-with-session ./run.sh
// ```
//
// where `run.sh` unsets `WAYLAND_DISPLAY` (so the probe takes the X11
// backend) and runs this test binary with `--ignored wm_`.

/// The window `_NET_ACTIVE_WINDOW` names, as `xdotool` spells ids (decimal).
fn active_window() -> String {
    let out = run("xprop", &["-root", "_NET_ACTIVE_WINDOW"]);
    let hex = out
        .rsplit("# ")
        .next()
        .map(str::trim)
        .unwrap_or_else(|| panic!("no _NET_ACTIVE_WINDOW: {out}"));
    u64::from_str_radix(hex.trim_start_matches("0x"), 16)
        .unwrap_or_else(|_| panic!("unreadable _NET_ACTIVE_WINDOW: {out}"))
        .to_string()
}

fn wm_moved_and_resized_comes_back(envs: &[(&str, &str)]) -> (Probe, String) {
    let wm = run("xprop", &["-root", "_NET_SUPPORTING_WM_CHECK"]);
    assert!(
        wm.contains("window id"),
        "positive control: a window manager runs on this display: {wm}"
    );
    let mut probe = Probe::spawn(envs);
    let wid = wait_for("the probe's window", visible_window);
    std::thread::sleep(Duration::from_millis(500));
    run("xdotool", &["windowmove", &wid, "300", "378"]);
    run("xdotool", &["windowsize", &wid, "700", "200"]);
    std::thread::sleep(Duration::from_millis(700));
    let before = geometry(&wid);
    assert_eq!(
        (before.2, before.3),
        (700, 200),
        "positive control: the resize reached the window"
    );
    assert!(
        before.0 > 250 && before.1 > 330,
        "positive control: the move reached the window: {before:?}"
    );
    let shown = hide_and_show(&mut probe);
    assert_eq!(
        settle(&shown, before),
        before,
        "the re-shown window is where it was hidden, at its size"
    );
    (probe, shown)
}

#[test]
#[ignore = "needs an X server with a window manager; see the section docs"]
fn wm_an_ordinary_window_comes_back_at_its_geometry_and_active() {
    let (_probe, shown) = wm_moved_and_resized_comes_back(&[]);
    let active = wait_for("the re-shown window to be activated", || {
        let a = active_window();
        (a == shown).then_some(a)
    });
    assert_eq!(active, shown, "an ordinary show still activates");
}

#[test]
#[ignore = "needs an X server with a window manager; see the section docs"]
fn wm_a_non_activating_window_comes_back_at_its_geometry_inactive() {
    let (_probe, shown) = wm_moved_and_resized_comes_back(&[("PROBE_NON_ACTIVATING", "1")]);
    std::thread::sleep(Duration::from_millis(700));
    assert_ne!(
        active_window(),
        shown,
        "the re-shown panel was not activated"
    );
    let hints = run("xprop", &["-id", &shown, "WM_HINTS"]);
    assert!(
        hints.contains("Client accepts input or input focus: False"),
        "{hints}"
    );
}
