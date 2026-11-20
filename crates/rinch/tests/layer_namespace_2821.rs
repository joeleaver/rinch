//! Issue #2821 on a live compositor: the namespace a layer surface is made
//! with, read back from the probe's own `get_layer_surface` request
//! (`examples/layer_commit_probe.rs` under `WAYLAND_DEBUG=client`, as
//! `layer_one_commit_2464.rs` runs it): the surface's own, else the app id;
//! kept by a live surface whatever the app gives it later, with one warning;
//! taken by a hidden window.
//!
//! Needs a compositor with `wlr-layer-shell`, so every test is `#[ignore]`d.
//! Run it inside an isolated session (`scripts/isolated-session.sh`, issue
//! #2429) on the session's own headless KWin, never on your desktop:
//!
//! ```sh
//! cargo build -p rinch --example layer_commit_probe
//! cargo test -p rinch --test layer_namespace_2821 --no-run
//! scripts/isolated-session.sh --kwin -- \
//!   cargo test -p rinch --test layer_namespace_2821 -- --ignored --test-threads=1
//! ```
//!
//! A wrapper that needs the user's session bus (a `systemd-run --user`
//! memory cap, for one) goes around `scripts/isolated-session.sh`, not
//! inside it: the session removes that bus.
#![cfg(all(feature = "windowing", target_os = "linux"))]

#[path = "support/layer_probe.rs"]
mod layer_probe;

use layer_probe::{Probe, layer_namespace, request_of};

/// What the shell logs when it keeps a live surface's namespace.
const KEPT: &str = "given to a live layer surface is ignored";

fn namespaces(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| layer_namespace(line))
        .collect()
}

fn warnings(lines: &[String]) -> usize {
    lines.iter().filter(|line| line.contains(KEPT)).count()
}

fn sent(lines: &[String], request: &str) -> bool {
    lines
        .iter()
        .any(|line| request_of(line).as_deref() == Some(request))
}

#[test]
fn the_namespace_is_read_from_a_request_line() {
    assert_eq!(
        layer_namespace(
            "[1158677.414]  -> zwlr_layer_shell_v1#9.get_layer_surface(new id \
             zwlr_layer_surface_v1#31, wl_surface#30, nil, 2, \"dock\")"
        )
        .as_deref(),
        Some("dock")
    );
    assert_eq!(
        layer_namespace("[1.0] -> zwlr_layer_surface_v1#31.set_size(800, 32)"),
        None
    );
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_surface_is_made_with_its_own_namespace() {
    let mut probe = Probe::start(&[
        ("PROBE_NAMESPACE", "dock"),
        ("PROBE_APP_ID", "org.example.Probe"),
    ]);
    let lines = probe.mapped();
    probe.quit();
    assert_eq!(namespaces(&lines), ["dock"]);
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn without_one_it_is_made_with_the_app_id() {
    let mut probe = Probe::start(&[("PROBE_APP_ID", "org.example.Probe")]);
    let lines = probe.mapped();
    probe.quit();
    assert_eq!(namespaces(&lines), ["org.example.Probe"]);
}

/// A live surface given another namespace with another exclusive zone takes
/// the zone, is not made again, and warns once; a second such change warns
/// no more; a change of output then makes it again, under the kept name.
#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_live_surface_keeps_its_namespace() {
    let mut probe = Probe::start(&[("PROBE_NAMESPACE", "dock")]);
    assert_eq!(namespaces(&probe.mapped()), ["dock"]);

    let renamed = probe.run("rename-and-zone");
    assert!(
        namespaces(&renamed).is_empty(),
        "not made again: {renamed:#?}"
    );
    assert!(
        sent(&renamed, "zwlr_layer_surface_v1.set_exclusive_zone"),
        "the zone changed: {renamed:#?}"
    );
    assert_eq!(warnings(&renamed), 1, "{renamed:#?}");

    let again = probe.run("rename-again");
    assert!(namespaces(&again).is_empty(), "{again:#?}");
    assert!(sent(&again, "zwlr_layer_surface_v1.set_exclusive_zone"));
    assert_eq!(warnings(&again), 0, "warned once only: {again:#?}");

    let moved = probe.run("rename-and-output");
    probe.quit();
    assert_eq!(
        namespaces(&moved),
        ["dock"],
        "made again for the output, under the kept name: {moved:#?}"
    );
    assert_eq!(warnings(&moved), 0, "{moved:#?}");
}

#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_hidden_window_is_made_with_a_new_namespace() {
    let mut probe = Probe::start(&[("PROBE_NAMESPACE", "dock")]);
    assert_eq!(namespaces(&probe.mapped()), ["dock"]);
    let shown = probe.run("hide-rename-show");
    probe.quit();
    assert_eq!(namespaces(&shown), ["desktop"], "{shown:#?}");
    assert_eq!(warnings(&shown), 0, "{shown:#?}");
}

/// A window renamed while hidden is made under the new name; a refusal on
/// that surface is warned of again, once.
#[test]
#[ignore = "needs a Wayland compositor with wlr-layer-shell; see module docs"]
fn a_surface_made_under_a_new_name_warns_again() {
    let mut probe = Probe::start(&[("PROBE_NAMESPACE", "dock")]);
    probe.mapped();
    let first = probe.run("rename-and-zone");
    assert_eq!(warnings(&first), 1, "{first:#?}");
    let shown = probe.run("hide-rename-show");
    assert_eq!(namespaces(&shown), ["desktop"], "{shown:#?}");
    let refused = probe.run("rename-again");
    assert_eq!(
        warnings(&refused),
        1,
        "the desktop surface's refusal: {refused:#?}"
    );
    let repeat = probe.run("rename-again");
    probe.quit();
    assert_eq!(warnings(&repeat), 0, "once on that surface: {repeat:#?}");
}
