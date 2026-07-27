//! Pins arboard's `wayland-data-control` feature into the build graph (#148).
//!
//! Without it, all Linux clipboard traffic goes through X11/XWayland, and
//! arboard's X11 backend serves its own cached copy for as long as it owns the
//! X11 selection — a compositor that doesn't sync Wayland→X11 (observed on KDE
//! Plasma Wayland) never revokes that ownership, so `paste_text()` silently
//! returns stale data. This test fails if a future dependency cleanup drops the
//! feature again.

#![cfg(target_os = "linux")]

use std::process::Command;

/// Asserts via `cargo tree -e features` that this crate enables arboard's
/// `wayland-data-control` feature and that `wl-clipboard-rs` (the native
/// Wayland backend it gates) is actually in the dependency graph.
#[test]
fn arboard_wayland_data_control_is_enabled() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["tree", "-p", "rinch-clipboard", "-e", "features"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to run `cargo tree`");
    assert!(
        output.status.success(),
        "`cargo tree` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(
        tree.contains("arboard feature \"wayland-data-control\""),
        "arboard's wayland-data-control feature is no longer enabled — \
         Linux clipboard would silently fall back to X11-only and serve \
         stale pastes on Wayland (#148)"
    );
    assert!(
        tree.contains("wl-clipboard-rs"),
        "wl-clipboard-rs (arboard's native Wayland backend) missing from \
         the dependency graph (#148)"
    );
}
