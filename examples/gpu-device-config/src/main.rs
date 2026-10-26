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
        required_features: wgpu::Features::empty(),
        required_limits: required_limits(),
    };

    match std::env::var("RINCH_GPU_MODE").as_deref() {
        Ok("external") => run_external(app, props),
        Ok("zerocopy") => {
            // gpu_config device + a surface drawn on it with no readback.
            App::new(zerocopy_app)
                .window_props(props)
                .gpu_config(gpu)
                .run();
        }
        // Default: rinch owns the device, requested with our raised limits.
        _ => App::new(app).window_props(props).gpu_config(gpu).run(),
    }
}

/// Build the whole GPU stack ourselves (the embedder-owns-the-device path) and
/// hand it to rinch. This mirrors an app like `arvx-render` that keeps its exact
/// device descriptor.
fn run_external(app: fn(&mut RenderScope) -> NodeHandle, props: WindowProps) {
    use std::sync::Arc;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env()
            .unwrap_or(wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12),
        ..Default::default()
    });
    // Headless adapter — no surface yet (rinch owns the window). On a single-GPU
    // system this is the same physical device that can present to that window.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
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
