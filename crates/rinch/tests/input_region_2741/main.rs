//! Issue #2741 on a live compositor: a transparent overlay with
//! `InputRegion::Painted` takes the pointer only where it paints, so a click
//! in the gap between two popups (or past a popup's rounded corner) reaches
//! the window underneath; and the region of a frame goes in that frame's own
//! commit when the overlay grows with its content.
//!
//! Each test starts one headless sway, a virtual pointer
//! ([`pointer::Pointer`]), and two clients of the probe
//! (`examples/input_region_probe.rs`): `under`, an 800x600 window at the
//! output's top-left that prints `UNDER CLICK`, and `overlay`, a
//! content-sized layer surface at (100, 100) holding two 200x60 popups 20px
//! apart, which prints `POPUP CLICK <n>` or `OVERLAY CLICK`.
//!
//! - [`all_takes_the_gap`] is today's behaviour, `InputRegion::All`: the gap
//!   click goes to the overlay and the window below hears nothing.
//! - [`painted_passes_the_gap_to_the_window_below`] is the fix: the popups
//!   still take their clicks; the gap, a rounded corner's outside and the
//!   space below the column go to `under`; after `grow` the third popup
//!   takes its click.
//! - [`the_region_goes_in_the_commit_of_its_frame`] reads the overlay's
//!   `WAYLAND_DEBUG` around `grow`: the region with the third popup is set
//!   in the commit that attaches the first buffer of the grown height, and
//!   no commit before it carries a region.
//!
//! Needs sway, so every test is `#[ignore]`d. Run them inside an isolated
//! session (`scripts/isolated-session.sh`, issue #2429), never on your
//! desktop:
//!
//! ```sh
//! cargo build -p rinch --example input_region_probe
//! cargo test -p rinch --test input_region_2741 --no-run
//! scripts/isolated-session.sh -- \
//!   cargo test -p rinch --test input_region_2741 -- --ignored --test-threads=1
//! ```
#![cfg(all(feature = "windowing", target_os = "linux"))]

mod pointer;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(30);
const OUTPUT: (u32, u32) = (1280, 800);

fn session_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RINCH_SESSION_DIR")
            .expect("run inside scripts/isolated-session.sh (RINCH_SESSION_DIR is not set)"),
    )
}

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"))
}

/// A headless sway with one output, killed on drop.
struct Sway {
    child: Child,
    display: String,
}

impl Sway {
    fn start(name: &str) -> Self {
        let runtime = runtime_dir();
        let config = session_dir().join(format!("sway-{name}.conf"));
        std::fs::write(
            &config,
            format!(
                "output HEADLESS-1 resolution {}x{} scale 1\n\
                 for_window [title=\"under\"] floating enable, move position 0 0\n",
                OUTPUT.0, OUTPUT.1
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

    fn socket(&self) -> PathBuf {
        runtime_dir().join(&self.display)
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
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
    let mut probe = profile_dir.join("examples").join("input_region_probe");
    if !probe.is_file() {
        // A separate build directory (`CARGO_BUILD_BUILD_DIR`) keeps the
        // test binary there and puts the example in the target directory.
        probe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(profile_dir.file_name().expect("a profile directory"))
            .join("examples")
            .join("input_region_probe");
    }
    assert!(
        probe.is_file(),
        "probe not found at {} — build it first:\n  \
         cargo build -p rinch --example input_region_probe",
        probe.display()
    );
    probe
}

/// A running probe client: its stdout and stderr lines, and its stdin.
struct Probe {
    child: Child,
    stdin: ChildStdin,
    out: Receiver<String>,
    err: Receiver<String>,
}

impl Probe {
    fn start(sway: &Sway, mode: &str, region: &str) -> Self {
        let mut child = Command::new(probe_binary())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("WAYLAND_DISPLAY", &sway.display)
            .env("MODE", mode)
            .env("REGION", region)
            .env("RINCH_RENDERER", "software")
            .env("RINCH_DEBUG", "0")
            .env("WAYLAND_DEBUG", "client")
            .spawn()
            .expect("spawn the probe");
        let stdin = child.stdin.take().unwrap();
        let out = lines(child.stdout.take().unwrap());
        let err = lines(child.stderr.take().unwrap());
        let probe = Self {
            child,
            stdin,
            out,
            err,
        };
        probe.wait_for_err("READY");
        probe
    }

    /// Wait for a stderr line equal to `line`; the lines before it.
    fn wait_for_err(&self, line: &str) -> Vec<String> {
        let mut seen = Vec::new();
        loop {
            let l = self
                .err
                .recv_timeout(PATIENCE)
                .unwrap_or_else(|_| panic!("no `{line}` from the probe"));
            if l == line {
                return seen;
            }
            seen.push(l);
        }
    }

    /// The stdout lines printed so far.
    fn clicks(&self) -> Vec<String> {
        self.out.try_iter().collect()
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn lines(r: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(r).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// Click at `(x, y)` and say who heard it: the under window's lines and the
/// overlay's.
fn click(
    pointer: &mut pointer::Pointer,
    under: &Probe,
    overlay: &Probe,
    x: u32,
    y: u32,
) -> (Vec<String>, Vec<String>) {
    pointer.click(x, y);
    std::thread::sleep(Duration::from_millis(400));
    (under.clicks(), overlay.clicks())
}

fn setup(name: &str, region: &str) -> (Sway, pointer::Pointer, Probe, Probe) {
    let sway = Sway::start(name);
    let pointer = pointer::Pointer::new(&sway.socket(), OUTPUT);
    let under = Probe::start(&sway, "under", region);
    let overlay = Probe::start(&sway, "overlay", region);
    // Let both settle on screen.
    std::thread::sleep(Duration::from_millis(500));
    let _ = (under.clicks(), overlay.clicks());
    (sway, pointer, under, overlay)
}

const GAP: (u32, u32) = (200, 170);
const POPUP0: (u32, u32) = (200, 130);
const POPUP1: (u32, u32) = (200, 210);

#[test]
#[ignore = "needs sway; see module docs"]
fn all_takes_the_gap() {
    let (_sway, mut pointer, under, overlay) = setup("all", "all");
    let (u, o) = click(&mut pointer, &under, &overlay, POPUP0.0, POPUP0.1);
    assert!(
        u.is_empty() && o.contains(&"POPUP CLICK 0".to_string()),
        "{u:?} {o:?}"
    );
    let (u, o) = click(&mut pointer, &under, &overlay, GAP.0, GAP.1);
    assert!(u.is_empty(), "the window below heard the gap click: {u:?}");
    assert_eq!(
        o,
        vec!["OVERLAY CLICK".to_string()],
        "the overlay swallows the gap click"
    );
}

#[test]
#[ignore = "needs sway; see module docs"]
fn painted_passes_the_gap_to_the_window_below() {
    let (_sway, mut pointer, under, mut overlay) = setup("painted", "painted");
    let (u, o) = click(&mut pointer, &under, &overlay, POPUP0.0, POPUP0.1);
    assert!(
        u.is_empty() && o.contains(&"POPUP CLICK 0".to_string()),
        "{u:?} {o:?}"
    );
    let (u, o) = click(&mut pointer, &under, &overlay, GAP.0, GAP.1);
    assert_eq!(
        u,
        vec!["UNDER CLICK".to_string()],
        "the gap click reaches the window below"
    );
    assert!(o.is_empty(), "the overlay heard the gap click: {o:?}");
    let (u, o) = click(&mut pointer, &under, &overlay, POPUP1.0, POPUP1.1);
    assert!(
        u.is_empty() && o.contains(&"POPUP CLICK 1".to_string()),
        "{u:?} {o:?}"
    );
    // Past the first popup's rounded top-left corner (radius 10).
    let (u, o) = click(&mut pointer, &under, &overlay, 101, 101);
    assert_eq!(
        u,
        vec!["UNDER CLICK".to_string()],
        "outside the corner: {o:?}"
    );
    // Where the third popup will be.
    let (u, o) = click(&mut pointer, &under, &overlay, 200, 290);
    assert_eq!(
        u,
        vec!["UNDER CLICK".to_string()],
        "below the column: {o:?}"
    );

    overlay.send("grow");
    overlay.wait_for_err("PROBE-END grow");
    let (u, o) = click(&mut pointer, &under, &overlay, 200, 290);
    assert!(
        u.is_empty() && o.contains(&"POPUP CLICK 2".to_string()),
        "{u:?} {o:?}"
    );
    let (u, _) = click(&mut pointer, &under, &overlay, GAP.0, GAP.1);
    assert_eq!(
        u,
        vec!["UNDER CLICK".to_string()],
        "the gap still passes after the resize"
    );
}

/// One `wl_surface.commit` of the overlay's surface: what went with it.
#[derive(Debug, Default, Clone)]
struct Commit {
    /// The height of the buffer it attached, if it attached one.
    attached_height: Option<i32>,
    /// The rects (`wl_region.add`) of the input region it carried, if any.
    region: Option<Vec<(i32, i32, i32, i32)>>,
}

/// The overlay surface's commits in `lines` (`WAYLAND_DEBUG=client`).
fn commits(lines: &[String]) -> Vec<Commit> {
    use std::collections::HashMap;
    let mut buffers: HashMap<String, i32> = HashMap::new();
    let mut regions: HashMap<String, Vec<(i32, i32, i32, i32)>> = HashMap::new();
    let mut pending = Commit::default();
    let mut out = Vec::new();
    let mut surface: Option<String> = None;
    for line in lines {
        let Some(req) = line.split(" -> ").nth(1) else {
            continue;
        };
        let (object, rest) = match req.split_once('.') {
            Some(p) => p,
            None => continue,
        };
        let object = object.trim().to_string();
        let args = rest
            .split_once('(')
            .map(|(_, a)| a.trim_end_matches(')').to_string())
            .unwrap_or_default();
        let name = rest.split('(').next().unwrap_or("");
        let arg_list: Vec<&str> = args.split(", ").collect();
        let num = |i: usize| arg_list.get(i).and_then(|a| a.trim().parse::<i32>().ok());
        match name {
            "create_buffer" => {
                // create_buffer(new id wl_buffer#N, offset, width, height, stride, format)
                if let (Some(id), Some(h)) = (arg_list.first(), num(3)) {
                    let id = id.trim_start_matches("new id ").to_string();
                    buffers.insert(id, h);
                }
            }
            "add" if object.starts_with("wl_region") => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (num(0), num(1), num(2), num(3)) {
                    regions
                        .entry(object.clone())
                        .or_default()
                        .push((x, y, w, h));
                }
            }
            "set_input_region" => {
                surface = Some(object.clone());
                let region = arg_list[0].trim();
                pending.region = Some(if region == "nil" {
                    vec![(i32::MIN, 0, 0, 0)]
                } else {
                    regions.get(region).cloned().unwrap_or_default()
                });
            }
            "attach"
                if object.starts_with("wl_surface")
                    && surface.as_deref().is_none_or(|s| s == object) =>
            {
                pending.attached_height = buffers.get(arg_list[0].trim()).copied();
            }
            "commit"
                if object.starts_with("wl_surface")
                    && surface.as_deref().is_none_or(|s| s == object) =>
            {
                out.push(std::mem::take(&mut pending));
            }
            _ => {}
        }
    }
    out
}

#[test]
#[ignore = "needs sway; see module docs"]
fn the_region_goes_in_the_commit_of_its_frame() {
    let (_sway, _pointer, _under, mut overlay) = setup("commit", "painted");
    // Everything up to now, to know the surface and its buffers.
    let mut log: Vec<String> = overlay.err.try_iter().collect();
    overlay.send("grow");
    let mut during = overlay.wait_for_err("PROBE-BEGIN grow");
    log.append(&mut during);
    let begin = log.len();
    let mut rest = overlay.wait_for_err("PROBE-END grow");
    log.append(&mut rest);
    let all = commits(&log);
    let before = commits(&log[..begin]).len();
    let after = &all[before..];
    // The third popup: 200x60 at y = 2 * (60 + 20) = 160; the gap above it
    // is 140..160.
    let covers = |c: &Commit, x: i32, y: i32| {
        c.region.as_ref().is_some_and(|r| {
            r.iter()
                .any(|&(rx, ry, w, h)| x >= rx && x < rx + w && y >= ry && y < ry + h)
        })
    };
    let has_third = |c: &Commit| covers(c, 100, 190) && !covers(c, 100, 150);
    let first_grown = after
        .iter()
        .position(|c| c.attached_height == Some(220))
        .unwrap_or_else(|| panic!("no 220px buffer after `grow`: {after:?}"));
    assert!(
        has_third(&after[first_grown]),
        "the first 220px frame does not carry the region with the third popup: {after:?}"
    );
    assert!(
        after[..first_grown].iter().all(|c| c.region.is_none()),
        "a commit before the grown frame carried a region: {after:?}"
    );
    assert_eq!(
        after.iter().filter(|c| c.region.is_some()).count(),
        1,
        "the region went out once: {after:?}"
    );
}

/// A click-through toggle takes effect with no frame after it: the empty
/// region is committed at once. Before #2741 it was set and not committed,
/// so a window that drew nothing new went on taking clicks.
#[test]
#[ignore = "needs sway; see module docs"]
fn a_click_through_toggle_needs_no_frame() {
    let (_sway, mut pointer, under, mut overlay) = setup("through", "all");
    let (u, o) = click(&mut pointer, &under, &overlay, POPUP0.0, POPUP0.1);
    assert!(
        u.is_empty() && o.contains(&"POPUP CLICK 0".to_string()),
        "{u:?} {o:?}"
    );
    overlay.send("through");
    overlay.wait_for_err("PROBE-BEGIN through");
    std::thread::sleep(Duration::from_millis(300));
    let (u, o) = click(&mut pointer, &under, &overlay, POPUP0.0, POPUP0.1);
    assert_eq!(
        u,
        vec!["UNDER CLICK".to_string()],
        "click-through is on: {o:?}"
    );
}

#[test]
fn parser_pairs_regions_with_their_commits() {
    let l = |s: &str| s.to_string();
    let lines = [
        l("[1.0]  -> wl_shm_pool#9.create_buffer(new id wl_buffer#30, 0, 200, 220, 800, 0)"),
        l("[1.0]  -> wl_compositor#3.create_region(new id wl_region#31)"),
        l("[1.0]  -> wl_region#31.add(0, 160, 200, 60)"),
        l("[1.0]  -> wl_surface#24.set_input_region(wl_region#31)"),
        l("[1.0]  -> wl_surface#24.attach(wl_buffer#30, 0, 0)"),
        l("[1.0]  -> wl_surface#24.commit()"),
        l("[1.0]  -> wl_surface#24.commit()"),
    ];
    let c = commits(&lines);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].attached_height, Some(220));
    assert_eq!(c[0].region, Some(vec![(0, 160, 200, 60)]));
    assert!(c[1].region.is_none() && c[1].attached_height.is_none());
}
