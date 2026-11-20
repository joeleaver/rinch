//! #2688 through the embed API: a host that renders the overlay several
//! times before the GPU catches up (B frames back to back, then waits) has at
//! most `gpu_frames_in_flight` of them in flight, so the wgpu-hal command
//! encoders alive at once, which wgpu and the driver keep for good, stay at
//! that many frames' worth.
//!
//! They need an adapter, so they are `#[ignore]`d, and `gpu` for `pollster`
//! (the probe only prints, and CI skips it):
//!     cargo test -p rinch --features gpu --test embed_frames_in_flight_2688 -- --ignored --test-threads=1

#![cfg(feature = "gpu-renderer")]

use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};
use rinch::prelude::*;

const W: u32 = 1280;
const H: u32 = 720;

fn context() -> RinchContext {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: W,
            height: H,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_inner_html(
                "<div style=\"position: absolute; inset: 0; font: 16px/20px sans-serif\">\
                 <p>An overlay, rendered back to back.</p>\
                 <div style=\"width: 200px; height: 100px; background: rgb(0,0,255); \
                 border-radius: 12px\"></div></div>",
            );
            root
        },
    );
    ctx.update(&[]);
    ctx
}

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a GPU adapter");
    eprintln!("#2688 adapter: {:?}", adapter.get_info().name);
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

/// This process's GTT in KiB (`drm-total-gtt` over its DRM clients), or 0
/// where none is reported.
fn gtt_kib() -> u64 {
    let mut total = 0;
    let mut clients = std::collections::HashSet::new();
    for entry in std::fs::read_dir("/proc/self/fdinfo")
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(info) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(client) = info.lines().find(|l| l.starts_with("drm-client-id:")) else {
            continue;
        };
        if !clients.insert(client.to_owned()) {
            continue;
        }
        for v in info
            .lines()
            .filter_map(|l| l.strip_prefix("drm-total-gtt:"))
        {
            let mut words = v.split_whitespace();
            let n: u64 = words.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            total += if words.next() == Some("MiB") {
                n * 1024
            } else {
                n
            };
        }
    }
    total
}

/// The most hal command encoders alive at once over `rounds` rounds of
/// `burst` overlay frames rendered back to back, each round then waited for.
fn peak_encoders(limit: usize, burst: usize, rounds: usize) -> isize {
    rinch::set_gpu_frames_in_flight(limit);
    let (device, queue) = gpu();
    let mut ctx = context();
    let mut overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let mut peak = 0;
    for _ in 0..rounds {
        for _ in 0..burst {
            ctx.update(&[]);
            let _view = overlay.render(&device, &queue, &mut ctx);
            peak = peak.max(device.get_internal_counters().hal.command_encoders.read());
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    eprintln!(
        "#2688 embed, limit {limit}, bursts of {burst}: GTT {} KiB",
        gtt_kib()
    );
    peak
}

/// One overlay frame has two encoders in flight (Vello's and wgpu's
/// pending-writes one for Vello's uploads) and one more ready (the
/// pending-writes encoder that replaced it): three. Each frame more in flight
/// adds two.
#[test]
#[ignore]
fn gpu_embed_frames_in_flight_are_limited_2688() {
    for (limit, most) in [(1, 3), (2, 5)] {
        let peak = peak_encoders(limit, 8, 6);
        eprintln!("#2688 embed, limit {limit}, bursts of 8: at most {peak} hal encoders");
        assert!(
            peak <= most,
            "{peak} hal encoders alive at once with {limit} overlay frames in flight"
        );
    }
}

/// No limit, for comparison (not asserted: how far frames overlap depends on
/// the GPU and the build).
#[test]
#[ignore]
fn embed_no_frame_limit_probe_2688() {
    for limit in [0, 2, 1] {
        for burst in [1, 2, 4, 8] {
            let peak = peak_encoders(limit, burst, 6);
            eprintln!("#2688 embed, limit {limit}, bursts of {burst}: at most {peak} hal encoders");
        }
    }
}

/// What the limit costs the CPU (run by hand, under `perf stat -e
/// instructions:u`, in a release build): `RINCH_2688_OP` is `poll` (a
/// `Device::poll(Poll)` with nothing in flight, what a frame that does not wait
/// adds), `wait` (a `Poll::Wait` for a finished submission), `submit` (an
/// empty submission, what an overlay frame without a split viewport adds) or
/// `none`, `RINCH_2688_N` times; or `render` at `RINCH_2688_LIMIT`, each
/// overlay frame waited for. Prints the wall time per call.
#[test]
#[ignore]
fn frame_cost_probe_2688() {
    let op = std::env::var("RINCH_2688_OP").unwrap_or_else(|_| "none".into());
    let n: usize = std::env::var("RINCH_2688_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let limit: usize = std::env::var("RINCH_2688_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    rinch::set_gpu_frames_in_flight(limit);
    let (device, queue) = gpu();
    let mut ctx = context();
    let mut overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let _ = overlay.render(&device, &queue, &mut ctx);
    let last = queue.submit(None);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let start = std::time::Instant::now();
    for _ in 0..n {
        match op.as_str() {
            "poll" => {
                let _ = device.poll(wgpu::PollType::Poll);
            }
            "wait" => {
                let _ = device.poll(wgpu::PollType::Wait {
                    submission_index: Some(last.clone()),
                    timeout: Some(std::time::Duration::from_secs(1)),
                });
            }
            "submit" => {
                queue.submit(None);
            }
            "render" => {
                let _ = overlay.render(&device, &queue, &mut ctx);
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
            }
            _ => {}
        }
    }
    eprintln!(
        "#2688 {op} x{n} (limit {limit}): {:?} each",
        start.elapsed() / n as u32
    );
}

/// The overlay keeps its frames in flight at the limit (#2688): rendered back
/// to back, it never has more than `limit` unfinished. It holds even where
/// frames barely overlap (lavapipe), since it reads the overlay's own count.
#[test]
#[ignore]
fn gpu_the_overlay_waits_at_its_limit_2688() {
    let (device, queue) = gpu();
    let mut ctx = context();
    for limit in [1, 2] {
        rinch::set_gpu_frames_in_flight(limit);
        let mut overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
        for _ in 0..16 {
            let _ = overlay.render(&device, &queue, &mut ctx);
            assert!(
                overlay.frames_in_flight_pending() <= limit,
                "{} overlay frames in flight at a limit of {limit}",
                overlay.frames_in_flight_pending()
            );
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
    rinch::set_gpu_frames_in_flight(2);
}
