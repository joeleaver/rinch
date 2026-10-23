//! A `RenderSurface` drawn with wgpu through `set_gpu_render_callback` (#1768).
//!
//! rinch hands the callback its own device, a target texture at the surface's
//! physical size, and the target's format; the frame is drawn in the scene as
//! the surface element's content, so the rounded panel clips it and the page's
//! background is beneath it, like any element's. The surface runs every frame while "Animate" is on (`set_continuous(true)`) and
//! only on request otherwise. Under the software renderer
//! (`RINCH_RENDERER=cpu`) the ordinary render callback draws a CPU fallback.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch::render_surface::{GpuSurfaceFrame, RenderSurface, create_render_surface};
use rinch::wgpu;

const SHADER: &str = r#"
struct U { angle: f32, _p0: f32, _p1: f32, _p2: f32 };
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array<vec2<f32>, 3>(vec2(0.0, 0.7), vec2(-0.6, -0.5), vec2(0.6, -0.5));
    let c = cos(u.angle);
    let s = sin(u.angle);
    let v = p[i];
    return vec4(v.x * c - v.y * s, v.x * s + v.y * c, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4(0.29, 0.82, 0.60, 1.0);
}
"#;

/// Pipeline state, rebuilt when the device or the target's format is new.
struct Gpu {
    generation: u64,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Gpu {
    fn new(f: &GpuSurfaceFrame<'_>) -> Self {
        let shader = f.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu-surface"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = f
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gpu-surface"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(f.format.into())],
                }),
                multiview: None,
                cache: None,
            });
        let uniform = f.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu-surface uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = f.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Gpu {
            generation: f.generation,
            format: f.format,
            pipeline,
            uniform,
            bind_group,
        }
    }

    fn draw(&self, f: &GpuSurfaceFrame<'_>, angle: f32) {
        let bytes: Vec<u8> = [angle, 0.0, 0.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        f.queue.write_buffer(&self.uniform, 0, &bytes);
        let mut encoder = f.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpu-surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: f.target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.06,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        f.queue.submit(Some(encoder.finish()));
    }
}

#[component]
fn app() -> NodeHandle {
    let surface = create_render_surface();
    let animate = Signal::new(true);
    let frames = Signal::new(0u64);
    surface.set_continuous(true);

    let gpu: Rc<RefCell<Option<Gpu>>> = Rc::default();
    surface.set_gpu_render_callback(move |f| {
        let mut gpu = gpu.borrow_mut();
        if gpu
            .as_ref()
            .is_none_or(|g| g.generation != f.generation || g.format != f.format)
        {
            *gpu = Some(Gpu::new(f));
        }
        let n = frames.get() + 1;
        frames.set(n);
        gpu.as_ref().unwrap().draw(f, n as f32 * 0.02);
    });
    // The software renderer's fallback: a flat CPU frame.
    surface.set_render_callback(move |writer, w, h| {
        let px = [26u8, 30, 46, 255].repeat((w * h) as usize);
        writer.submit_frame(&px, w, h);
    });

    let toggle = surface.clone();
    let step = surface.clone();
    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100vh; background: #f4f5f7; color: #222; font-family: sans-serif;",
            div {
                style: "display: flex; gap: 12px; align-items: center; padding: 12px;",
                Button {
                    onclick: move || {
                        let on = !animate.get();
                        animate.set(on);
                        toggle.set_continuous(on);
                    },
                    {|| if animate.get() { "Pause" } else { "Animate" }}
                }
                Button { onclick: move || step.request_frame(), "One frame" }
                // Hiding destroys the window and showing makes a new one; the
                // surface keeps drawing on the same device (#1768).
                Button {
                    onclick: move || {
                        hide_current_window();
                        set_timeout(800, show_current_window);
                    },
                    "Hide for a moment"
                }
                span { {|| format!("frames drawn: {}", frames.get())} }
            }
            div {
                style: "flex: 1; margin: 0 12px 12px; border-radius: 8px; overflow: hidden;",
                RenderSurface { surface: Some(surface) }
            }
        }
    }
}

fn main() {
    App::new(app)
        .title("GPU RenderSurface")
        .size(640, 480)
        .run();
}
