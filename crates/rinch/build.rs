//! Emits the `software_shell` cfg alias (issue #140).
//!
//! `software_shell` is set when a native shell can present via the TinySkia
//! software painter: every `windowing` build (`desktop` included), and an
//! `android` build that the GPU shell (`android-gpu`) did not replace. Painter
//! selection is **additive**: the Vello painter is gated on
//! `any(gpu-renderer, android-gpu, embed)` and the software painter on
//! `software_shell`, so `windowing` + `embed` carries BOTH painters (the winit
//! shell drives `build_pixels`/TinySkia while embed `RinchContext`s drive
//! `build_scene`/Vello), and so does `windowing` + `gpu-renderer` (or `gpu`),
//! whose winit shell picks one of the two when it creates the window
//! and falls back to the software one when the GPU cannot start
//! (`shell::renderer`). One alias here instead of repeating the compound
//! condition at every gate in `app/`.
//!
//! "Is a GPU compositor presenting?" is therefore a runtime question on a
//! `gpu-renderer` build: ask `shell::renderer::gpu_presenting()`, never
//! `cfg!(not(software_shell))`.

//!
//! Two more for the GPU render callback (#1768): `web_gpu` is the browser
//! with the `web-gpu` feature (`wasm32-unknown-unknown`; the feature does
//! nothing elsewhere), and `surface_gpu` is either half of the API, desktop
//! (`gpu-renderer`) or web (`web_gpu`), for the types and methods both share.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(software_shell)");
    println!("cargo::rustc-check-cfg=cfg(web_gpu)");
    println!("cargo::rustc-check-cfg=cfg(surface_gpu)");

    let feature = |name: &str| std::env::var(format!("CARGO_FEATURE_{name}")).is_ok();
    let target = |key: &str| std::env::var(format!("CARGO_CFG_TARGET_{key}")).unwrap_or_default();
    if feature("WINDOWING") || (feature("ANDROID") && !feature("ANDROID_GPU")) {
        println!("cargo::rustc-cfg=software_shell");
    }
    let web_gpu = feature("WEB_GPU") && target("ARCH") == "wasm32" && target("OS") == "unknown";
    if web_gpu {
        println!("cargo::rustc-cfg=web_gpu");
    }
    if web_gpu || feature("GPU_RENDERER") {
        println!("cargo::rustc-cfg=surface_gpu");
    }
}
