//! Issue #2761: `App::try_run` answers a typed error when there is no
//! display, or the display goes away, *before* the event loop runs. It used
//! to panic there ("Failed to create event loop", status 101), where the same
//! display going away a moment later was `RunError::DisplayLost`.
//!
//! No compositor is needed. Each case runs this binary again as a child on
//! its main thread (winit refuses to build an event loop on any other, which
//! is why this target has `harness = false`), with the display variables
//! pointing at something that is not a display server:
//!
//! - nothing: no `WAYLAND_DISPLAY`, `WAYLAND_SOCKET` or `DISPLAY`;
//! - a `WAYLAND_DISPLAY` naming a socket that does not exist;
//! - a `DISPLAY` naming an X server that does not exist;
//! - `prefer_x11` with both of those stale (X11 refused, then the default);
//! - `prefer_x11` with only the stale `DISPLAY` (no second X11 attempt);
//! - a "compositor" that accepts the connection and closes it at once: the
//!   display was there and went away before the loop ran, as when a
//!   compositor exits in the gap the issue describes.
//!
//! The first five are `RunError::NoDisplay`, the last `RunError::DisplayLost`.
//! `App::run` on no display logs one line and exits with status 1, as it does
//! for an error the running loop ends with, rather than panicking.
//!
//! Each child prints the `Debug` of what `try_run` returned, so the parent
//! reads it from stdout: a child that panicked (the bug) prints nothing.

#[cfg(all(target_os = "linux", feature = "windowing"))]
mod linux {
    use rinch::prelude::*;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    const CHILD: &str = "RINCH_2761_CHILD";
    const RESULT: &str = "RINCH-2761-RESULT: ";

    #[component]
    fn nothing() -> NodeHandle {
        __scope.create_element("div")
    }

    /// In the child: start the app the parent named, print what `try_run`
    /// returned (or, for `run`, nothing: it does not return on an error).
    pub fn child(which: &str) {
        match which {
            "windowless" => {
                let result = App::windowless(nothing).try_run();
                println!("{RESULT}{result:?}");
            }
            "windowed" => {
                let result = App::new(nothing).try_run();
                println!("{RESULT}{result:?}");
            }
            "prefer-x11" => {
                let result = App::windowless(nothing).prefer_x11(true).try_run();
                println!("{RESULT}{result:?}");
            }
            "run" => {
                App::windowless(nothing).run();
                println!("{RESULT}run returned");
            }
            other => panic!("unknown child {other}"),
        }
    }

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("rinch-2761-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        Dir(dir)
    }

    /// An X display number nothing serves: no socket for it in
    /// `/tmp/.X11-unix`.
    fn stale_x_display() -> String {
        let n = (27610..27700)
            .find(|n| !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists())
            .expect("a free X display number");
        format!(":{n}")
    }

    fn run_child(which: &str, runtime_dir: &Path, env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(std::env::current_exe().expect("this binary's path"));
        cmd.env(CHILD, which)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .env_remove("RINCH_DEBUG")
            .env_remove("RUST_LOG")
            .env("XDG_RUNTIME_DIR", runtime_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("the test binary re-runs itself")
    }

    /// Every case runs; the failures are reported together at the end.
    #[derive(Default)]
    struct Report(Vec<String>);

    impl Report {
        /// The child's `try_run` returned `Err(variant(..))`. A child that
        /// printed no result (it panicked) fails with its output.
        fn expect(&mut self, case: &str, out: &Output, variant: &str) {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let result = stdout.lines().find_map(|l| l.strip_prefix(RESULT));
            match result {
                Some(r) if r.starts_with(&format!("Err({variant}(")) && out.status.success() => {
                    println!("ok   {case}: {r}");
                }
                _ => self.fail(format!(
                    "{case}: expected {variant}, got {result:?} (status {:?}):\n{stderr}",
                    out.status.code()
                )),
            }
        }

        fn fail(&mut self, message: String) {
            println!("FAIL {message}");
            self.0.push(message);
        }
    }

    pub fn parent() {
        let mut report = Report::default();
        let runtime = temp_dir("runtime");
        let rt = runtime.0.as_path();
        let stale_x = stale_x_display();

        // Nothing to connect to.
        for which in ["windowless", "windowed"] {
            let out = run_child(which, rt, &[]);
            report.expect(
                &format!("no display variables ({which})"),
                &out,
                "NoDisplay",
            );
        }

        // A WAYLAND_DISPLAY naming no socket.
        let out = run_child(
            "windowless",
            rt,
            &[("WAYLAND_DISPLAY", "no-such-wayland-2761")],
        );
        report.expect("stale WAYLAND_DISPLAY", &out, "NoDisplay");

        // A DISPLAY naming no X server.
        let out = run_child("windowless", rt, &[("DISPLAY", &stale_x)]);
        report.expect("stale DISPLAY", &out, "NoDisplay");

        // prefer_x11: X11 refused (a warning), then the default backend,
        // which has nothing either.
        let out = run_child(
            "prefer-x11",
            rt,
            &[
                ("DISPLAY", &stale_x),
                ("WAYLAND_DISPLAY", "no-such-wayland-2761"),
            ],
        );
        report.expect("prefer_x11, both stale", &out, "NoDisplay");

        // prefer_x11 in an X11-only session whose X server is gone: the
        // default backend is X11 again, which winit will not build twice
        // (it answered "EventLoop can't be recreated").
        let out = run_child("prefer-x11", rt, &[("DISPLAY", &stale_x)]);
        report.expect("prefer_x11, stale DISPLAY only", &out, "NoDisplay");

        // A compositor that takes the connection and goes away before the
        // loop runs: accepted, then closed without a word.
        let socket = rt.join("gone-2761");
        let listener = UnixListener::bind(&socket).expect("bind the fake compositor");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        // Polled, so a child that never connects cannot hang the test.
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while std::time::Instant::now() < deadline {
                if let Ok((stream, _)) = listener.accept() {
                    drop(stream);
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            false
        });
        let out = run_child("windowless", rt, &[("WAYLAND_DISPLAY", "gone-2761")]);
        if !server.join().expect("the fake compositor's thread") {
            report.fail("the child never connected to the fake compositor".into());
        }
        report.expect("compositor gone after connecting", &out, "DisplayLost");

        // `run`: one line, status 1, no panic.
        let out = run_child("run", rt, &[]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.code() == Some(1)
            && !stderr.contains("panicked")
            && stderr.contains("no display server")
        {
            println!("ok   run on no display: status 1, one error line");
        } else {
            report.fail(format!(
                "run on no display: expected status 1 and an error line, got status {:?}:\n{stderr}",
                out.status.code()
            ));
        }

        assert!(
            report.0.is_empty(),
            "{} of 8 cases failed:\n{}",
            report.0.len(),
            report.0.join("\n")
        );
    }
}

fn main() {
    #[cfg(all(target_os = "linux", feature = "windowing"))]
    {
        if let Ok(which) = std::env::var("RINCH_2761_CHILD") {
            linux::child(&which);
            return;
        }
        // libtest's arguments (`--list`, a filter) are not ours: listing
        // shows nothing, and any run runs every case.
        if std::env::args().any(|a| a == "--list") {
            return;
        }
        linux::parent();
    }
}
