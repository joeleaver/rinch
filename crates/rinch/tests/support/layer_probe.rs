//! The layer-surface probe (`examples/layer_commit_probe.rs`) run under
//! `WAYLAND_DEBUG=client`, which prints every request it sends: started,
//! waited for until its surface is mapped, driven one stdin command at a
//! time, and its stderr read between the probe's markers. Shared by the
//! live layer-shell tests (`layer_one_commit_2464.rs`,
//! `layer_namespace_2821.rs`), which run inside `scripts/isolated-session.sh
//! --kwin`.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// The probe, running: its stdin, and its stderr's lines as they come.
pub struct Probe {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Probe {
    /// The probe started with `env` set, with the software renderer and no
    /// debug server.
    pub fn start(env: &[(&str, &str)]) -> Self {
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
        for (name, value) in env {
            cmd.env(name, value);
        }
        let mut child = cmd.spawn().expect("spawn the probe");
        let stdin = child.stdin.take().unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stderr.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn next(&self, what: &str) -> String {
        self.lines
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| panic!("the probe printed nothing more while waiting for {what}"))
    }

    /// Its stderr until its surface is mapped (its first frame went out: a
    /// commit after the first configure), then for half a second more,
    /// so that its startup traffic has settled.
    pub fn mapped(&mut self) -> Vec<String> {
        let mut seen = Vec::new();
        let mut configured = false;
        loop {
            let line = self.next("its first frame");
            let request = request_of(&line);
            seen.push(line);
            match request.as_deref() {
                Some("zwlr_layer_surface_v1.ack_configure") => configured = true,
                Some("wl_surface.commit") if configured => break,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        seen.extend(self.lines.try_iter());
        seen
    }

    /// Its stderr between its markers for `command`.
    pub fn run(&mut self, command: &str) -> Vec<String> {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
        let begin = format!("PROBE-BEGIN {command}");
        let end = format!("PROBE-END {command}");
        while self.next(&begin) != begin {}
        let mut lines = Vec::new();
        loop {
            let line = self.next(&end);
            if line == end {
                return lines;
            }
            lines.push(line);
        }
    }

    /// Asks it to quit, and waits for it.
    pub fn quit(mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.child.wait();
    }
}

/// The probe example's binary: beside this test binary's profile directory,
/// or, with Cargo's `build.build-dir` set (intermediate artifacts, test
/// binaries among them, elsewhere), in the workspace's `target/<profile>/`,
/// where Cargo puts final artifacts such as examples.
fn probe_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("this test binary's own path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("a test binary lives in <target>/<profile>/deps/");
    let profile = profile_dir
        .file_name()
        .expect("the profile directory has a name");
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    let candidates = [
        profile_dir.join("examples").join("layer_commit_probe"),
        target
            .join(profile)
            .join("examples")
            .join("layer_commit_probe"),
    ];
    candidates
        .iter()
        .find(|probe| probe.is_file())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "probe not found at {candidates:?} — build it first:\n  \
                 cargo build -p rinch --example layer_commit_probe"
            )
        })
}

/// `interface.request` from a `WAYLAND_DEBUG` request line, `None` for
/// anything else (an event). libwayland writes an object as `wl_surface#7`
/// (`[1158677.414]  -> wl_surface#7.commit()`), wayland-rs's own backend as
/// `wl_surface@7`.
pub fn request_of(line: &str) -> Option<String> {
    let rest = line.split(" -> ").nth(1)?;
    let (interface, rest) = rest.split_once(['#', '@'])?;
    let request = rest.split_once('.')?.1.split_once('(')?.0;
    Some(format!("{}.{request}", interface.trim()))
}

/// The namespace of a `zwlr_layer_shell_v1.get_layer_surface` request
/// line, its last argument
/// (`-> zwlr_layer_shell_v1#9.get_layer_surface(new id zwlr_layer_surface_v1#31, wl_surface#30, nil, 2, "dock")`);
/// `None` for any other line.
pub fn layer_namespace(line: &str) -> Option<String> {
    if request_of(line).as_deref() != Some("zwlr_layer_shell_v1.get_layer_surface") {
        return None;
    }
    let args = line.rsplit_once(')')?.0;
    let (before, _) = args.rsplit_once('"')?;
    let (_, namespace) = before.rsplit_once('"')?;
    Some(namespace.to_string())
}
