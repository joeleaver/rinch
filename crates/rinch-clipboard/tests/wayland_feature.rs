//! Pins arboard's `wayland-data-control` feature into the build graph (#148).
//!
//! Without it, all Linux clipboard traffic goes through X11/XWayland, and
//! arboard's X11 backend serves its own cached copy for as long as it owns the
//! X11 selection — a compositor that doesn't sync Wayland→X11 (observed on KDE
//! Plasma Wayland) never revokes that ownership, so `paste_text()` silently
//! returns stale data. This test fails if a future dependency cleanup drops the
//! feature again.
//!
//! **How it asks, and why not `cargo tree` (#275).** arboard exposes no public
//! item behind the feature, so the question cannot be put to the type system.
//! It used to be put to cargo instead — `cargo tree -e features`, spawned from
//! the test with `current_dir(env!("CARGO_MANIFEST_DIR"))`. That path is baked
//! in at compile time, and a shared target dir hands a later `cargo test` the
//! binary a since-deleted git worktree built, so the spawn failed with
//! `NotFound` for its *working directory* — deterministically, while looking
//! like a flake. Now the test asks the arboard that is actually linked into
//! this binary: with the feature, `Clipboard::new()` tries the Wayland
//! data-control backend first whenever `WAYLAND_DISPLAY` is set, and logs a
//! warning through `log` when that fails; without it, it never tries and logs
//! nothing about Wayland. Pointing `WAYLAND_DISPLAY` at a socket that does not
//! exist makes the attempt fail on every host, so no compositor, display or
//! subprocess is involved, and nothing is read from disk.

#![cfg(target_os = "linux")]

use std::sync::Mutex;

/// Every record logged in this process, as `(target, message)`.
static RECORDS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        RECORDS
            .lock()
            .unwrap()
            .push((record.target().to_string(), record.args().to_string()));
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

#[test]
fn arboard_wayland_data_control_is_enabled() {
    log::set_logger(&CAPTURE).expect("the only logger in this test binary");
    log::set_max_level(log::LevelFilter::Trace);

    // Positive control: the instrument hears a record at all. Without it, a
    // `log` built with a static max level (`release_max_level_off`, say) would
    // make the assertion below fail for the wrong reason — and a future
    // rewrite that inverts it would pass for one.
    log::warn!(target: "wayland_feature_probe", "probe");
    assert!(
        RECORDS
            .lock()
            .unwrap()
            .iter()
            .any(|(target, _)| target == "wayland_feature_probe"),
        "the capture logger heard nothing; this test cannot see arboard's log"
    );

    // A socket name that cannot exist, and no X11 display: the Wayland attempt
    // fails, the X11 fallback fails, and neither touches the developer's
    // session. This file holds one test, so nothing races the environment.
    std::env::set_var("WAYLAND_DISPLAY", "rinch-275-no-such-wayland-socket");
    std::env::remove_var("DISPLAY");
    let _ = arboard::Clipboard::new();

    let records = RECORDS.lock().unwrap().clone();
    assert!(
        records
            .iter()
            .any(|(target, message)| target.starts_with("arboard")
                && message.to_ascii_lowercase().contains("wayland")),
        "arboard never tried its Wayland data-control backend, so its \
         `wayland-data-control` feature is no longer enabled — Linux clipboard \
         would silently fall back to X11-only and serve stale pastes on \
         Wayland (#148). Records heard: {records:?}"
    );
}
