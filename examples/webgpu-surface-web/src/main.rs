//! WebGPU `RenderSurface` demo for `rinch-web` (issues #91, #1768).
//!
//! Rinch owns the chrome (the left panel) as **real browser DOM** and the
//! `<canvas>` the surface draws into, which it sizes, HiDPI-scales and wires
//! for input. The triangle is drawn through `set_gpu_render_callback`, the
//! same API and the same code as the desktop's `examples/gpu-surface`: rinch
//! requests the page's WebGPU device (the `web-gpu` feature) and hands the
//! callback the device, its queue and the canvas's current texture at the
//! canvas's device-pixel size. The browser composites the DOM and the canvas.
//!
//! The surface runs every frame while "Animate" is on (`set_continuous(true)`)
//! and only on request otherwise. Where the browser has no WebGPU, the GPU
//! callback is not run, `gpu_frames_available()` stays false, and the ordinary
//! render callback draws a CPU fallback with `submit_frame`.
//!
//! Build & run:
//! ```bash
//! cd examples/webgpu-surface-web
//! trunk serve --release --port 8080
//! ```
//! Serve over `http://localhost` (or https) so `navigator.gpu` is exposed.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch::render_surface::{GpuSurfaceFrame, RenderSurface, SurfaceEvent, create_render_surface};
use rinch::wgpu;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::prelude::*;

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
            label: Some("webgpu-surface-web"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = f
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("webgpu-surface-web"),
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
                multiview_mask: None,
                cache: None,
            });
        let uniform = f.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("webgpu-surface-web uniform"),
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
                label: Some("webgpu-surface-web"),
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
                multiview_mask: None,
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
    let gpu_state = Signal::new(String::from("waiting for the device"));
    let target = Signal::new(String::from("—"));
    let last_event = Signal::new(String::from("(click the canvas, then type)"));
    surface.set_continuous(true);

    // The GPU path: the same callback as the desktop example's.
    let gpu: Rc<RefCell<Option<Gpu>>> = Rc::default();
    surface.set_gpu_render_callback(move |f| {
        let mut gpu = gpu.borrow_mut();
        if gpu
            .as_ref()
            .is_none_or(|g| g.generation != f.generation || g.format != f.format)
        {
            *gpu = Some(Gpu::new(f));
            gpu_state.set(format!("WebGPU, device #{}, {:?}", f.generation, f.format));
        }
        let size = format!("{} × {} px", f.size.0, f.size.1);
        if target.get() != size {
            target.set(size);
        }
        let n = frames.get() + 1;
        frames.set(n);
        gpu.as_ref().unwrap().draw(f, n as f32 * 0.02);
    });
    // The fallback where the browser has no WebGPU: a flat CPU frame.
    surface.set_render_callback(move |writer, w, h| {
        if gpu_state.get() != "no WebGPU: CPU fallback" {
            gpu_state.set("no WebGPU: CPU fallback".into());
        }
        let px = [26u8, 30, 46, 255].repeat((w * h) as usize);
        writer.submit_frame(&px, w, h);
    });

    // Input over the canvas is delivered here (pointer / wheel / key / focus).
    surface.set_event_handler(move |ev| {
        let s = match ev {
            SurfaceEvent::MouseDown { x, y, button } => {
                format!("MouseDown {x:.0},{y:.0} {button:?}")
            }
            SurfaceEvent::MouseUp { x, y, button } => format!("MouseUp {x:.0},{y:.0} {button:?}"),
            SurfaceEvent::MouseMove { x, y } => format!("MouseMove {x:.0},{y:.0}"),
            SurfaceEvent::MouseWheel {
                delta_x, delta_y, ..
            } => format!("Wheel {delta_x:.0},{delta_y:.0}"),
            SurfaceEvent::KeyDown(k) => format!("KeyDown {:?}", k.key),
            SurfaceEvent::KeyUp(k) => format!("KeyUp {:?}", k.key),
            SurfaceEvent::FocusGained => "FocusGained".into(),
            SurfaceEvent::FocusLost => "FocusLost".into(),
            other => format!("{other:?}"),
        };
        last_event.set(s);
    });

    let toggle = surface.clone();
    let step = surface.clone();
    rsx! {
        div {
            style: "display:flex; height:100vh; width:100vw; \
                    font-family: system-ui, -apple-system, sans-serif; \
                    color:#e7e7ea; background:#111318;",

            // ── rinch DOM chrome (real DOM, composited AROUND the canvas) ──
            div {
                style: "width:300px; padding:22px; display:flex; flex-direction:column; \
                        gap:14px; background:#181b22; border-right:1px solid #262a33;",
                h1 { style:"margin:0; font-size:18px;", "WebGPU RenderSurface" }
                p {
                    style:"margin:0; font-size:13px; color:#9aa0ab; line-height:1.55;",
                    "This panel is real rinch DOM. The triangle is drawn by a GPU render \
                     callback into the canvas's current texture, on the WebGPU device \
                     rinch requested for the page."
                }
                div {
                    style:"display:flex; gap:8px;",
                    Button {
                        onclick: move || {
                            let on = !animate.get();
                            animate.set(on);
                            toggle.set_continuous(on);
                        },
                        {|| if animate.get() { "Pause" } else { "Animate" }}
                    }
                    Button { onclick: move || step.request_frame(), "One frame" }
                }
                div { style:"height:1px; background:#262a33;" }
                div { style:"font-size:12px; color:#9aa0ab;", "Drawing with" }
                div { style:"font-size:15px; color:#7fd7a6;", {|| gpu_state.get()} }
                div { style:"font-size:12px; color:#9aa0ab;", "GPU frame target" }
                div {
                    style:"font-size:15px; color:#7fd7a6; font-variant-numeric:tabular-nums;",
                    {|| target.get()}
                }
                div { style:"font-size:12px; color:#9aa0ab;", "Frames drawn" }
                div {
                    style:"font-size:15px; color:#7fd7a6; font-variant-numeric:tabular-nums;",
                    {|| frames.get().to_string()}
                }
                div { style:"font-size:12px; color:#9aa0ab; margin-top:6px;", "Last canvas event" }
                div {
                    style:"font-size:15px; color:#7fb2ff; font-variant-numeric:tabular-nums;",
                    {|| last_event.get()}
                }
                div { style:"flex:1;" }
                p {
                    style:"margin:0; font-size:11px; color:#6b7180; line-height:1.5;",
                    "Pause stops the frames; One frame draws one. Resize the window: \
                     the target follows the canvas's device-pixel size."
                }
            }

            // ── the surface ────────────────────────────────────────────────
            div {
                style: "flex:1; position:relative; min-width:0;",
                RenderSurface { surface: Some(surface) }
            }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();
    let theme = ThemeProviderProps {
        dark_mode: true,
        ..Default::default()
    };
    rinch_web::mount(theme, app);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
}
