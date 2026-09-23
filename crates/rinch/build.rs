//! Emits the `software_shell` cfg alias (issue #140).
//!
//! `software_shell` is set when a native shell can present via the TinySkia
//! software painter: every `desktop` build, and an `android` build that the
//! GPU shell (`android-gpu`) did not replace. Painter selection is
//! **additive**: the Vello painter is gated on `any(gpu, android-gpu, embed)`
//! and the software painter on `software_shell`, so `desktop` + `embed`
//! carries BOTH painters (the winit shell drives `build_pixels`/TinySkia while
//! embed `RinchContext`s drive `build_scene`/Vello), and so does `desktop` +
//! `gpu`, whose winit shell picks one of the two when it creates the window
//! and falls back to the software one when the GPU cannot start
//! (`shell::renderer`). One alias here instead of repeating the compound
//! condition at every gate in `app/`.
//!
//! "Is a GPU compositor presenting?" is therefore a runtime question on a
//! `desktop` + `gpu` build: ask `shell::renderer::gpu_presenting()`, never
//! `cfg!(not(software_shell))`.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(software_shell)");

    let feature = |name: &str| std::env::var(format!("CARGO_FEATURE_{name}")).is_ok();
    if feature("DESKTOP") || (feature("ANDROID") && !feature("ANDROID_GPU")) {
        println!("cargo::rustc-cfg=software_shell");
    }
}
