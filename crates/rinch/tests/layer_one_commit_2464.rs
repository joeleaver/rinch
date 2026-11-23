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
//!
//! A wrapper that needs the user's session bus (a `systemd-run --user`
//! memory cap, for one) goes around `scripts/isolated-session.sh`, not
//! inside it: the session removes that bus.
#![cfg(all(feature = "windowing", target_os = "linux"))]

#[path = "support/layer_probe.rs"]
mod layer_probe;

use layer_probe::{Probe, request_of};

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

/// The requests (`interface.request`, in order) the probe sent between its
/// markers for `command`.
fn requests_during(overlay: bool, command: &str) -> Vec<String> {
    requests_with(
        if overlay {
            &[("PROBE_OVERLAY", "1")]
        } else {
            &[]
        },
        command,
    )
}

/// [`requests_during`] for a probe started with `env`.
fn requests_with(env: &[(&str, &str)], command: &str) -> Vec<String> {
    let mut probe = Probe::start(env);
    probe.mapped();
    let requests = probe
        .run(command)
        .iter()
        .filter_map(|line| request_of(line))
        .collect();
    probe.quit();
    requests
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

/// #2829: a panel sized by its content (#2552) given a new margin, which
/// narrows its content, takes its new size in the same commit as the
/// margin: the drain stages the content's size with the layer changes,
/// where `follow_content_size` used to ask it after them, a second commit.
#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_content_sized_panel_given_a_margin_is_one_commit() {
    let requests = requests_with(&[("PROBE_CONTENT", "1")], "content-margin");
    let (commits, sent) = layer_commits(&requests);
    for w in ["set_margin", "set_size"] {
        assert!(
            sent.iter().any(|s| s == w),
            "content-margin: no {w} in {requests:?}"
        );
    }
    assert_eq!(
        commits, 1,
        "content-margin: layer state went out in {commits} commits: {requests:?}"
    );
}
