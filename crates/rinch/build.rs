//! Emits the `software_shell` cfg alias (issue #140).
//!
//! `software_shell` is set when a native shell (`desktop` or `android`)
//! presents via the TinySkia software painter — i.e. no GPU shell feature
//! (`gpu` / `android-gpu`) replaced it. Painter selection is **additive**:
//! the Vello painter is gated on `any(gpu, android-gpu, embed)` and the
//! software painter on `software_shell`, so `desktop` (software) + `embed`
//! carries BOTH painters — the winit shell drives `build_pixels`/TinySkia
//! while embed `RinchContext`s drive `build_scene`/Vello. One alias here
//! instead of repeating the compound condition at every gate in `app/`.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(software_shell)");

    let feature = |name: &str| std::env::var(format!("CARGO_FEATURE_{name}")).is_ok();
    let native_shell = feature("DESKTOP") || feature("ANDROID");
    let gpu_shell = feature("GPU") || feature("ANDROID_GPU");
    if native_shell && !gpu_shell {
        println!("cargo::rustc-cfg=software_shell");
    }
}
