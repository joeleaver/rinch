//! Probe for issue #2843: the shared memory a software-rendered wallpaper
//! holds, as `RssShmem` in `/proc/<pid>/status`, before and after its idle
//! trim.
//!
//! A `wlr-layer-shell` background surface on every edge of the output (a
//! wallpaper). It shows a colour, then changes it `PROBE_FRAMES - 1` times,
//! 300 ms apart, and draws nothing after that. Every second it prints
//! `RssShmem` to stderr with the time since start and the frames drawn.
//!
//! Environment: `PROBE_FRAMES` (default 3); `PROBE_IDLE_TRIM` = `never` or a
//! period in milliseconds (default: the default, 2 s); `PROBE_SECONDS`, how
//! long it runs (default 8); `PROBE_LATE_FRAME_MS`, one more colour that long
//! after start (the first frame after a trim). Run with `RINCH_RENDERER=software` on a
//! wlroots compositor.

use rinch::prelude::*;
use rinch_core::element::{
    IdleTrim, LayerAnchor, LayerAxes, LayerShellLayer, LayerSurface, WindowProps,
};
use std::time::{Duration, Instant};

const COLOURS: [&str; 4] = ["#36a", "#a63", "#3a6", "#63a"];

fn rss_shmem() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("RssShmem:"))
                .map(|l| l["RssShmem:".len()..].trim().to_string())
        })
        .unwrap_or_default()
}

#[component]
fn app() -> NodeHandle {
    let frames: usize = std::env::var("PROBE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let seconds: u64 = std::env::var("PROBE_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let colour = Signal::new(0usize);
    let start = Instant::now();
    for i in 1..frames {
        rinch::core::set_timeout(300 * i as u32, move || colour.set(i));
    }
    // One more frame this long after start, if set: the first after a trim.
    if let Some(ms) = std::env::var("PROBE_LATE_FRAME_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        rinch::core::set_timeout(ms, move || colour.set(frames));
    }
    for s in 1..=seconds {
        rinch::core::set_timeout(1000 * s as u32, move || {
            eprintln!(
                "PROBE t={:.1}s colour={} RssShmem={}",
                start.elapsed().as_secs_f32(),
                colour.get(),
                rss_shmem()
            );
            if s == seconds {
                std::process::exit(0);
            }
        });
    }
    rsx! {
        div {
            style: {move || format!(
                "width: 100%; height: 100%; background: {};",
                COLOURS[colour.get() % COLOURS.len()]
            )},
        }
    }
}

fn main() {
    let idle_trim = match std::env::var("PROBE_IDLE_TRIM").ok().as_deref() {
        Some("never") => IdleTrim::Never,
        Some(ms) => IdleTrim::After(Duration::from_millis(ms.parse().unwrap())),
        None => IdleTrim::default(),
    };
    eprintln!("PROBE pid={} idle_trim={idle_trim:?}", std::process::id());
    let props = WindowProps {
        title: "idle trim probe".into(),
        wayland_layer: Some(LayerSurface {
            layer: LayerShellLayer::Background,
            anchor: LayerAnchor::ALL,
            compositor_size: LayerAxes::BOTH,
            ..Default::default()
        }),
        idle_trim,
        ..Default::default()
    };
    App::new(app).window_props(props).run();
}
