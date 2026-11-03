//! Issue #57 — share rinch's compositor GPU device with the embedding app.
//!
//! An embedding application whose own renderer needs a higher-capability wgpu
//! device (extra features, larger storage buffers, more bind groups, …) can
//! raise those requirements so that the device rinch composites with — the one
//! published via [`gpu_handle`] — can also host the embedder's pipelines and
//! textures for **zero-copy** present (no GPU→CPU→GPU readback).
//!
//! This example proves the enabler two ways (pick with `RINCH_GPU_MODE`):
//!
//!   * **`config`** (default) — start rinch with [`App::gpu_config`],
//!     bumping `max_storage_buffers_per_shader_stage` from the wgpu default of 8
//!     to 32. rinch owns the device.
//!   * **`external`** — the embedder builds the whole GPU stack with its own
//!     `DeviceDescriptor` (as `arvx-render`'s `new_headless` does) and hands it
//!     to rinch via [`App::external_gpu`]. rinch composites onto that
//!     exact device.
//!
//! Either way the panel reads the *resolved* device back out of [`gpu_handle`]
//! and confirms the shared device carries the raised limit and that the
//! embedder can create GPU resources on it.
//!
//! `RINCH_GPU_MODE=zerocopy` draws with that device through a `RenderSurface`'s
//! GPU render callback (`RenderSurfaceHandle::set_gpu_render_callback`): the
//! callback renders into a texture on the shared device, and the window draws
//! it as the element's content, with no readback (composited outside the scene
//! when it can be, #1865).
//!
//! `RINCH_GPU_MODE=frames` draws that surface continuously and shows the
//! window's frame-to-frame time, with a button that turns vsync off and on
//! (`rinch::window::set_vsync`, #2095), and which of the optional device
//! features it asked for the adapter granted (`TIMESTAMP_QUERY`, #2085).
//! `RINCH_MEASURE_FRAMES=N` makes it measure instead: N frames with vsync on,
//! N with it off, a summary of each on stdout, and exit. `RINCH_VSYNC=off`
//! starts the default mode with vsync off.

use rinch::prelude::*;
// `rinch::wgpu` is the exact (patched) wgpu rinch links against — construct GPU
// config types from it so they match rinch's, not a separately-pinned wgpu.
use rinch::wgpu;

#[component]
fn app() -> NodeHandle {
    // The renderer is created before the component mounts, so the shared handle
    // is available here.
    let handle = gpu_handle().expect("gpu_handle available after the renderer starts");
    let device_limits = handle.device.limits();
    let adapter_info = handle.adapter.get_info();
    let has_f32_filterable = handle
        .device
        .features()
        .contains(wgpu::Features::FLOAT32_FILTERABLE);

    // Prove the embedder can allocate GPU resources on rinch's device.
    let probe = handle.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("embedder-probe-buffer"),
        size: 256,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let allocated_ok = probe.size() == 256;

    let rows = vec![
        ("adapter".to_string(), adapter_info.name.clone()),
        ("backend".to_string(), format!("{:?}", adapter_info.backend)),
        (
            "max_storage_buffers_per_shader_stage".to_string(),
            format!(
                "{} (wgpu default is 8)",
                device_limits.max_storage_buffers_per_shader_stage
            ),
        ),
        (
            "max_bind_groups".to_string(),
            device_limits.max_bind_groups.to_string(),
        ),
        (
            "FLOAT32_FILTERABLE feature".to_string(),
            has_f32_filterable.to_string(),
        ),
        (
            "allocated storage buffer on shared device".to_string(),
            if allocated_ok { "OK" } else { "FAILED" }.to_string(),
        ),
    ];

    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:12px; padding:24px; \
                    font-family: system-ui, sans-serif;",
            h1 { style: "margin:0; font-size:20px;", "Shared GPU device (issue #57)" }
            p {
                style: "margin:0; color:#555; font-size:13px;",
                "App::gpu_config raised the compositor device's limits. \
                 These values are read back from gpu_handle()."
            }
            div {
                style: "display:flex; flex-direction:column; gap:6px; margin-top:8px;",
                for row in rows.clone() {
                    div {
                        key: row.0.clone(),
                        style: "display:flex; gap:8px; font-size:14px;",
                        span { style: "min-width:340px; color:#333; font-weight:600;", {row.0.clone()} }
                        span { style: "color:#0a7;", {row.1.clone()} }
                    }
                }
            }
        }
    }
}

/// `RINCH_GPU_MODE=zerocopy` — prove ask #3: the same `App::gpu_config`
/// device (configurable features) ALSO draws a surface with no readback.
///
/// The surface's GPU render callback is handed a target texture on **rinch's
/// shared device** and clears it to a solid colour with a render pass; the
/// window draws that texture as the element's content, at its place in paint
/// order like an `<img>` (composited outside the scene when it can be, #1865),
/// through the exact device we raised the limits on.
#[component]
fn zerocopy_app() -> NodeHandle {
    use rinch::render_surface::create_render_surface;

    let surface = create_render_surface();
    surface.set_gpu_render_callback(|frame| {
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            // Solid teal, fully opaque: cleared straight into the target.
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("zerocopy-surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0x14 as f64 / 255.0,
                            g: 0xb8 as f64 / 255.0,
                            b: 0xa6 as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        frame.queue.submit([encoder.finish()]);
    });

    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:10px; padding:20px; \
                    font-family: system-ui, sans-serif;",
            h1 { style: "margin:0; font-size:18px;", "Zero-copy GPU surface (issue #57 ask #3)" }
            p {
                style: "margin:0; color:#333; font-size:13px;",
                "The teal box is drawn by a RenderSurface's GPU render callback into a \
                 texture on the App::gpu_config device, and drawn by the window \
                 directly — no CPU readback."
            }
            RenderSurface { surface: Some(surface), style: "width:320px; height:220px;" }
        }
    }
}

/// The elevated limits both modes request (the arvx headline: 8 → 32).
fn required_limits() -> wgpu::Limits {
    wgpu::Limits {
        max_storage_buffers_per_shader_stage: 32,
        ..Default::default()
    }
}

fn main() {
    let props = WindowProps {
        title: "GPU device config".into(),
        width: 720,
        height: 380,
        ..Default::default()
    };

    let gpu = RinchGpuConfig {
        required_limits: required_limits(),
        // Asked for, not required: an adapter without it still starts (#2085).
        optional_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    };

    match std::env::var("RINCH_GPU_MODE").as_deref() {
        Ok("external") => run_external(app, props),
        Ok("frames") => {
            App::new(frames_app)
                .window_props(props)
                .gpu_config(gpu)
                .run();
        }
        Ok("zerocopy") => {
            // gpu_config device + a surface drawn on it with no readback.
            App::new(zerocopy_app)
                .window_props(props)
                .gpu_config(gpu)
                .run();
        }
        // Default: rinch owns the device, requested with our raised limits.
        // `RINCH_VSYNC=off` starts it uncapped (#2095): with nothing asking
        // for frames, it still sleeps.
        _ => App::new(app)
            .window_props(props)
            .gpu_config(gpu)
            .vsync(std::env::var("RINCH_VSYNC").as_deref() != Ok("off"))
            .run(),
    }
}

/// Build the whole GPU stack ourselves (the embedder-owns-the-device path) and
/// hand it to rinch. This mirrors an app like `arvx-render` that keeps its exact
/// device descriptor.
fn run_external(app: fn(&mut RenderScope) -> NodeHandle, props: WindowProps) {
    use std::sync::Arc;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env()
            .unwrap_or(wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // Headless adapter — no surface yet (rinch owns the window). On a single-GPU
    // system this is the same physical device that can present to that window.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("no GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("embedder device"),
        required_features: wgpu::Features::empty(),
        required_limits: required_limits(),
        ..Default::default()
    }))
    .expect("failed to create embedder device");

    let gpu = ExternalGpu {
        instance,
        adapter: Arc::new(adapter),
        device: Arc::new(device),
        queue: Arc::new(queue),
    };
    App::new(app).window_props(props).external_gpu(gpu).run();
}

/// `RINCH_GPU_MODE=frames` — vsync off for measuring (#2095), and the optional
/// features the device was granted (#2085).
///
/// A continuous GPU surface asks for a frame every frame; the time between
/// two of its render callbacks is the window's frame time. With vsync on it
/// cannot go below the display's refresh interval; off, it is what a frame
/// costs.
#[component]
fn frames_app() -> NodeHandle {
    use rinch::render_surface::create_render_surface;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    let handle = gpu_handle().expect("gpu_handle available after the renderer starts");
    let granted = format!("{:?}", handle.granted_optional_features());
    let measure: Option<usize> = std::env::var("RINCH_MEASURE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok());

    let frame_ms = Signal::new(String::from("—"));
    let surface = create_render_surface();
    surface.set_continuous(true);
    let last: Rc<RefCell<Option<Instant>>> = Rc::default();
    let window: Rc<RefCell<Vec<f64>>> = Rc::default();
    let run = Rc::new(RefCell::new(Measurement::new(measure)));
    let mut frame = 0u64;
    surface.set_gpu_render_callback(move |gpu| {
        let now = Instant::now();
        if let Some(prev) = last.borrow_mut().replace(now) {
            let ms = now.duration_since(prev).as_secs_f64() * 1000.0;
            let mut w = window.borrow_mut();
            w.push(ms);
            if w.len() == 30 {
                let mean = w.iter().sum::<f64>() / w.len() as f64;
                frame_ms.set(format!("{mean:.2} ms ({:.0} fps)", 1000.0 / mean));
                w.clear();
            }
            run.borrow_mut().frame(ms);
        }
        frame += 1;
        let t = (frame % 120) as f64 / 120.0;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frames-surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: t,
                            g: 0.6,
                            b: 1.0 - t,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        gpu.queue.submit([encoder.finish()]);
    });

    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:10px; padding:20px; \
                    font-family: system-ui, sans-serif;",
            h1 { style: "margin:0; font-size:18px;", "Frame time and vsync (#2095, #2085)" }
            p { style: "margin:0; font-size:14px;",
                "Frame time: " {|| frame_ms.get()}
            }
            p { style: "margin:0; font-size:14px;",
                "Presentation: " {|| format!("{:?}", rinch::window::presentation())}
            }
            p { style: "margin:0; font-size:14px;",
                "Optional features granted: " {granted.clone()}
            }
            button {
                onclick: || {
                    let honoured = set_vsync(!rinch::window::vsync());
                    println!("set_vsync -> {honoured:?}");
                },
                {|| if rinch::window::vsync() { "Turn vsync off" } else { "Turn vsync on" }}
            }
            RenderSurface { surface: Some(surface), style: "width:320px; height:200px;" }
        }
    }
}

/// `RINCH_MEASURE_FRAMES=N`: N frames with vsync on, then N with it off, each
/// after a settling run of frames, summarised on stdout.
struct Measurement {
    n: usize,
    phase: usize,
    skip: usize,
    samples: Vec<f64>,
}

impl Measurement {
    const SETTLE: usize = 60;

    fn new(n: Option<usize>) -> Self {
        Self {
            n: n.unwrap_or(0),
            phase: 0,
            skip: Self::SETTLE,
            samples: Vec::new(),
        }
    }

    fn frame(&mut self, ms: f64) {
        if self.n == 0 {
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        self.samples.push(ms);
        if self.samples.len() < self.n {
            return;
        }
        let label = if self.phase == 0 {
            "vsync on "
        } else {
            "vsync off"
        };
        let mut s = std::mem::take(&mut self.samples);
        s.sort_by(f64::total_cmp);
        let mean = s.iter().sum::<f64>() / s.len() as f64;
        let pct = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
        println!(
            "{label} ({:?}): {} frames, mean {mean:.2} ms, min {:.2}, median {:.2}, p95 {:.2}, max {:.2}",
            rinch::window::presentation(),
            s.len(),
            s[0],
            pct(0.5),
            pct(0.95),
            s[s.len() - 1],
        );
        self.phase += 1;
        self.skip = Self::SETTLE;
        if self.phase == 1 {
            let honoured = set_vsync(false);
            println!("set_vsync(false) -> {honoured:?}");
        } else {
            std::process::exit(0);
        }
    }
}
