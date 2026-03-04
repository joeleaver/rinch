//! RenderSurface section — demonstrates embedding custom renderers.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rinch::prelude::*;
use rinch::render_surface::{SurfaceEvent, SurfaceMouseButton, create_render_surface};

// ── CPU 3D Math Helpers (desktop only) ─────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod cpu_cube {
    pub const CUBE_VERTS: [[f32; 3]; 8] = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];

    pub const CUBE_FACES: [([usize; 4], [u8; 4]); 6] = [
        ([4, 5, 6, 7], [66, 133, 244, 255]),
        ([1, 0, 3, 2], [234, 67, 53, 255]),
        ([0, 4, 7, 3], [52, 168, 83, 255]),
        ([5, 1, 2, 6], [251, 188, 4, 255]),
        ([7, 6, 2, 3], [255, 255, 255, 255]),
        ([0, 1, 5, 4], [255, 109, 0, 255]),
    ];

    pub fn rotate_y(v: [f32; 3], angle: f32) -> [f32; 3] {
        let (s, c) = angle.sin_cos();
        [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
    }

    pub fn rotate_x(v: [f32; 3], angle: f32) -> [f32; 3] {
        let (s, c) = angle.sin_cos();
        [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
    }

    #[derive(Copy, Clone)]
    pub struct Vertex {
        pub x: f32,
        pub y: f32,
        pub z: f32,
    }

    pub fn project(v: [f32; 3], w: f32, h: f32, zoom: f32) -> Vertex {
        let fov = 4.0;
        let d = v[2] + fov;
        let scale = (fov * zoom) / d;
        let base = w.min(h) * 0.5;
        Vertex {
            x: w * 0.5 + v[0] * scale * base,
            y: h * 0.5 - v[1] * scale * base,
            z: v[2],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_triangle_zbuf(
        buf: &mut [u8],
        zbuf: &mut [f32],
        w: i32,
        h: i32,
        v0: Vertex,
        v1: Vertex,
        v2: Vertex,
        color: [u8; 4],
    ) {
        let min_x = v0.x.min(v1.x).min(v2.x).max(0.0) as i32;
        let max_x = v0.x.max(v1.x).max(v2.x).min((w - 1) as f32) as i32;
        let min_y = v0.y.min(v1.y).min(v2.y).max(0.0) as i32;
        let max_y = v0.y.max(v1.y).max(v2.y).min((h - 1) as f32) as i32;

        let edge = |ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32| -> f32 {
            (bx - ax) * (py - ay) - (by - ay) * (px - ax)
        };

        let area = edge(v0.x, v0.y, v1.x, v1.y, v2.x, v2.y);
        if area.abs() < 0.001 {
            return;
        }
        let inv_area = 1.0 / area;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let w0 = edge(v1.x, v1.y, v2.x, v2.y, px, py);
                let w1 = edge(v2.x, v2.y, v0.x, v0.y, px, py);
                let w2 = edge(v0.x, v0.y, v1.x, v1.y, px, py);

                let inside = if area > 0.0 {
                    w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                } else {
                    w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                };

                if inside {
                    let b0 = (w0 * inv_area).abs();
                    let b1 = (w1 * inv_area).abs();
                    let b2 = (w2 * inv_area).abs();
                    let z = b0 * v0.z + b1 * v1.z + b2 * v2.z;

                    let pi = (y * w + x) as usize;
                    if z < zbuf[pi] {
                        zbuf[pi] = z;
                        let idx = pi * 4;
                        buf[idx] = color[0];
                        buf[idx + 1] = color[1];
                        buf[idx + 2] = color[2];
                        buf[idx + 3] = color[3];
                    }
                }
            }
        }
    }

    pub fn fill_quad_zbuf(
        buf: &mut [u8],
        zbuf: &mut [f32],
        w: i32,
        h: i32,
        v: [Vertex; 4],
        color: [u8; 4],
    ) {
        fill_triangle_zbuf(buf, zbuf, w, h, v[0], v[1], v[2], color);
        fill_triangle_zbuf(buf, zbuf, w, h, v[0], v[2], v[3], color);
    }
}

// ── wgpu Instanced Cube Wave (wasm32 only) ─────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod gpu_cube {
    use wgpu::util::DeviceExt;
    use wgpu::web_sys;

    // Column-major 4×4 matrix
    type Mat4 = [f32; 16];

    fn identity() -> Mat4 {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fovy * 0.5).tan();
        let nf = 1.0 / (near - far);
        [
            f / aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            (far + near) * nf,
            -1.0,
            0.0,
            0.0,
            2.0 * far * near * nf,
            0.0,
        ]
    }

    fn translate(m: &Mat4, x: f32, y: f32, z: f32) -> Mat4 {
        let mut out = *m;
        out[12] = m[0] * x + m[4] * y + m[8] * z + m[12];
        out[13] = m[1] * x + m[5] * y + m[9] * z + m[13];
        out[14] = m[2] * x + m[6] * y + m[10] * z + m[14];
        out[15] = m[3] * x + m[7] * y + m[11] * z + m[15];
        out
    }

    fn mat4_rotate_x(m: &Mat4, angle: f32) -> Mat4 {
        let (s, c) = angle.sin_cos();
        let mut out = *m;
        out[4] = m[4] * c + m[8] * s;
        out[5] = m[5] * c + m[9] * s;
        out[6] = m[6] * c + m[10] * s;
        out[7] = m[7] * c + m[11] * s;
        out[8] = m[8] * c - m[4] * s;
        out[9] = m[9] * c - m[5] * s;
        out[10] = m[10] * c - m[6] * s;
        out[11] = m[11] * c - m[7] * s;
        out
    }

    fn mat4_rotate_y(m: &Mat4, angle: f32) -> Mat4 {
        let (s, c) = angle.sin_cos();
        let mut out = *m;
        out[0] = m[0] * c - m[8] * s;
        out[1] = m[1] * c - m[9] * s;
        out[2] = m[2] * c - m[10] * s;
        out[3] = m[3] * c - m[11] * s;
        out[8] = m[0] * s + m[8] * c;
        out[9] = m[1] * s + m[9] * c;
        out[10] = m[2] * s + m[10] * c;
        out[11] = m[3] * s + m[11] * c;
        out
    }

    fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += a[k * 4 + row] * b[col * 4 + k];
                }
                out[col * 4 + row] = sum;
            }
        }
        out
    }

    const SHADER_SRC: &str = r#"
struct Uniforms {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    time: f32,
    grid_size: u32,
    spacing: f32,
    amplitude: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @builtin(instance_index) instance_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

fn hue_to_rgb(h: f32) -> vec3<f32> {
    let r = abs(h * 6.0 - 3.0) - 1.0;
    let g = 2.0 - abs(h * 6.0 - 2.0);
    let b = 2.0 - abs(h * 6.0 - 4.0);
    return saturate(vec3<f32>(r, g, b));
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let ix = i32(in.instance_index % uniforms.grid_size);
    let iz = i32(in.instance_index / uniforms.grid_size);
    let half = f32(uniforms.grid_size) * 0.5;

    let world_x = (f32(ix) - half) * uniforms.spacing;
    let world_z = (f32(iz) - half) * uniforms.spacing;

    // Layered sine waves for organic motion
    let wave1 = sin(world_x * 0.8 + uniforms.time * 2.0)
              * cos(world_z * 0.6 + uniforms.time * 1.5);
    let wave2 = sin(world_x * 0.3 - world_z * 0.5 + uniforms.time) * 0.5;
    let wave = wave1 + wave2;
    let world_y = wave * uniforms.amplitude;

    // Per-cube rotation around Y for visual richness
    let cube_angle = f32(in.instance_index) * 0.37 + uniforms.time * 0.8;
    let cs = cos(cube_angle);
    let sn = sin(cube_angle);
    let rotated = vec3<f32>(
        in.position.x * cs - in.position.z * sn,
        in.position.y,
        in.position.x * sn + in.position.z * cs,
    );

    let scale = uniforms.spacing * 0.35;
    let world_pos = vec3<f32>(world_x, world_y, world_z) + rotated * scale;

    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(world_pos, 1.0);

    // Rotate normal to match per-cube rotation, then by camera model
    let rot_normal = vec3<f32>(
        in.normal.x * cs - in.normal.z * sn,
        in.normal.y,
        in.normal.x * sn + in.normal.z * cs,
    );
    out.normal = normalize((uniforms.model * vec4<f32>(rot_normal, 0.0)).xyz);

    // Color from wave height — rainbow gradient
    let t = saturate((wave + 1.5) / 3.0);
    out.color = hue_to_rgb(t * 0.75 + 0.55);

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(0.3, 0.7, 0.5));
    let diff = max(dot(in.normal, light_dir), 0.0);
    let ambient = 0.3;
    let color = in.color * (ambient + diff * 0.7);
    return vec4<f32>(color, 1.0);
}
"#;

    const FACE_DEFS: [([usize; 4], [f32; 3]); 6] = [
        ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
        ([1, 0, 3, 2], [0.0, 0.0, -1.0]),
        ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),
        ([5, 1, 2, 6], [1.0, 0.0, 0.0]),
        ([7, 6, 2, 3], [0.0, 1.0, 0.0]),
        ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
    ];

    const CUBE_VERTS: [[f32; 3]; 8] = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];

    // Uniform buffer: MVP(64) + model(64) + time(4) + grid_size(4) + spacing(4) + amplitude(4) = 144
    const UNIFORM_SIZE: usize = 144;

    fn uniforms_to_bytes(
        mvp: &Mat4,
        model: &Mat4,
        time: f32,
        grid_size: u32,
        spacing: f32,
        amplitude: f32,
    ) -> [u8; UNIFORM_SIZE] {
        let mut buf = [0u8; UNIFORM_SIZE];
        for (i, &v) in mvp.iter().chain(model.iter()).enumerate() {
            buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        buf[128..132].copy_from_slice(&time.to_le_bytes());
        buf[132..136].copy_from_slice(&grid_size.to_le_bytes());
        buf[136..140].copy_from_slice(&spacing.to_le_bytes());
        buf[140..144].copy_from_slice(&amplitude.to_le_bytes());
        buf
    }

    pub struct GpuCube {
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        pipeline: wgpu::RenderPipeline,
        vertex_buffer: wgpu::Buffer,
        uniform_buffer: wgpu::Buffer,
        bind_group: wgpu::BindGroup,
        depth_view: wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        current_size: (u32, u32),
    }

    impl GpuCube {
        pub async fn new(canvas: web_sys::HtmlCanvasElement) -> Self {
            let instance = wgpu::util::new_instance_with_webgpu_detection(
                &wgpu::InstanceDescriptor::default(),
            )
            .await;

            let surface: wgpu::Surface<'static> = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
                .unwrap();

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .expect("No WebGPU or WebGL2 adapter found");

            let info = adapter.get_info();
            web_sys::console::log_1(
                &format!(
                    "RenderSurface: wgpu backend = {:?}, adapter = {}",
                    info.backend, info.name
                )
                .into(),
            );

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                    ..Default::default()
                })
                .await
                .unwrap();

            let caps = surface.get_capabilities(&adapter);
            let surface_format = caps.formats[0];

            // Vertex data: position(3) + normal(3) per vertex, 36 vertices
            let mut data: Vec<f32> = Vec::with_capacity(36 * 6);
            for (indices, normal) in &FACE_DEFS {
                for &tri in &[[0, 1, 2], [0, 2, 3]] {
                    for &vi in &tri {
                        let pos = CUBE_VERTS[indices[vi]];
                        data.extend_from_slice(&pos);
                        data.extend_from_slice(normal);
                    }
                }
            }

            let data_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr() as *const u8,
                    data.len() * std::mem::size_of::<f32>(),
                )
            };

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cube vertices"),
                contents: data_bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });

            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cube uniforms"),
                size: UNIFORM_SIZE as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cube wave shader"),
                source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cube wave pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 6 * 4, // position(3) + normal(3)
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x3,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x3,
                                offset: 12,
                                shader_location: 1,
                            },
                        ],
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Ccw,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24Plus,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });

            let depth_view = create_depth_view(&device, 1, 1);

            Self {
                device,
                queue,
                surface,
                pipeline,
                vertex_buffer,
                uniform_buffer,
                bind_group,
                depth_view,
                surface_format,
                current_size: (0, 0),
            }
        }

        pub fn render(
            &mut self,
            angle_x: f32,
            angle_y: f32,
            zoom: f32,
            time: f32,
            grid_size: u32,
            w: u32,
            h: u32,
        ) {
            if w == 0 || h == 0 {
                return;
            }

            if (w, h) != self.current_size {
                self.surface.configure(
                    &self.device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: self.surface_format,
                        width: w,
                        height: h,
                        present_mode: wgpu::PresentMode::AutoVsync,
                        alpha_mode: wgpu::CompositeAlphaMode::Auto,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    },
                );
                self.depth_view = create_depth_view(&self.device, w, h);
                self.current_size = (w, h);
            }

            let aspect = w as f32 / h.max(1) as f32;
            let proj = perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 200.0);

            let mut model = identity();
            model = translate(&model, 0.0, 0.0, -25.0 / zoom);
            model = mat4_rotate_x(&model, angle_x);
            model = mat4_rotate_y(&model, angle_y);

            let mvp = multiply(&proj, &model);

            let uniform_bytes = uniforms_to_bytes(&mvp, &model, time, grid_size, 0.5, 2.0);
            self.queue
                .write_buffer(&self.uniform_buffer, 0, &uniform_bytes);

            let frame = match self.surface.get_current_texture() {
                Ok(f) => f,
                Err(_) => return,
            };
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 24.0 / 255.0,
                                g: 24.0 / 255.0,
                                b: 32.0 / 255.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });

                let num_instances = grid_size * grid_size;
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..36, 0..num_instances);
            }

            self.queue.submit([encoder.finish()]);
            frame.present();
        }
    }

    fn create_depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
}

// ── Cube State ──────────────────────────────────────────────────────────────

struct CubeState {
    angle_x: f32,
    angle_y: f32,
    zoom: f32,
    dragging: bool,
    last_mouse: (f32, f32),
}

impl Default for CubeState {
    fn default() -> Self {
        Self {
            // On web: looking down at the wave; on desktop: slight tilt at single cube
            #[cfg(target_arch = "wasm32")]
            angle_x: 0.8,
            #[cfg(not(target_arch = "wasm32"))]
            angle_x: 0.4,
            angle_y: 0.0,
            zoom: 1.0,
            dragging: false,
            last_mouse: (0.0, 0.0),
        }
    }
}

// ── Section ─────────────────────────────────────────────────────────────────

#[component]
pub fn render_surface_section() -> NodeHandle {
    let spinning = Signal::new(true);
    let speed = Signal::new(1.0_f64);
    let grid_size = Signal::new(50_u32);
    let grid_size_f64 = Signal::new(50.0_f64);
    let fps = Signal::new(0_u32);

    // ── Cube Demo ───────────────────────────────────────────────────────

    let cube_surface = create_render_surface();
    let cube_state = Arc::new(Mutex::new(CubeState::default()));

    // Event handler for mouse interaction
    {
        let state = cube_state.clone();
        cube_surface.set_event_handler(move |event| {
            let mut s = state.lock().unwrap();
            match event {
                SurfaceEvent::MouseDown {
                    x,
                    y,
                    button: SurfaceMouseButton::Left,
                } => {
                    s.dragging = true;
                    s.last_mouse = (x, y);
                }
                SurfaceEvent::MouseUp {
                    button: SurfaceMouseButton::Left,
                    ..
                } => {
                    s.dragging = false;
                }
                SurfaceEvent::MouseMove { x, y } if s.dragging => {
                    let dx = x - s.last_mouse.0;
                    let dy = y - s.last_mouse.1;
                    s.angle_y += dx * 0.01;
                    s.angle_x += dy * 0.01;
                    s.last_mouse = (x, y);
                }
                SurfaceEvent::MouseWheel { delta_y, .. } => {
                    s.zoom = (s.zoom - delta_y * 0.001).clamp(0.3, 3.0);
                }
                _ => {}
            }
        });
    }

    // Render callback — runs every frame
    {
        let state = cube_state.clone();

        #[cfg(target_arch = "wasm32")]
        {
            use std::cell::RefCell;

            let gpu_state: Rc<RefCell<Option<gpu_cube::GpuCube>>> = Rc::new(RefCell::new(None));
            let init_started = Rc::new(Cell::new(false));
            let cube_surface_inner = cube_surface.clone();
            let time = Rc::new(Cell::new(0.0_f32));
            let frame_count = Rc::new(Cell::new(0_u32));
            let last_fps_time = Rc::new(Cell::new(0.0_f64));

            cube_surface.set_render_callback(move |_writer, w, h| {
                // FPS tracking
                let now = js_sys::Date::now() / 1000.0; // seconds
                let fc = frame_count.get() + 1;
                frame_count.set(fc);
                let elapsed = now - last_fps_time.get();
                if elapsed >= 1.0 {
                    fps.set((fc as f64 / elapsed) as u32);
                    frame_count.set(0);
                    last_fps_time.set(now);
                }
                let is_spinning = spinning.get();
                let spd = speed.get() as f32;
                let gs = grid_size.get();

                let mut s = state.lock().unwrap();
                if is_spinning {
                    s.angle_y += 0.005 * spd;
                }
                let angle_x = s.angle_x;
                let angle_y = s.angle_y;
                let zoom = s.zoom;
                drop(s);

                let t = time.get();
                time.set(t + 0.016 * spd);

                if let Ok(mut gc) = gpu_state.try_borrow_mut() {
                    if let Some(ref mut cube) = *gc {
                        cube.render(angle_x, angle_y, zoom, t, gs, w, h);
                        return;
                    }
                }

                if !init_started.get() {
                    if let Some(canvas) = cube_surface_inner.canvas_element() {
                        init_started.set(true);
                        let gpu_state_clone = gpu_state.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let cube = gpu_cube::GpuCube::new(canvas).await;
                            *gpu_state_clone.borrow_mut() = Some(cube);
                        });
                    }
                }
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use cpu_cube::*;
            use std::time::Instant;

            let last_fps_instant = Rc::new(Cell::new(Instant::now()));
            let frame_count = Rc::new(Cell::new(0_u32));

            cube_surface.set_render_callback(move |writer, w, h| {
                // FPS tracking
                let fc = frame_count.get() + 1;
                frame_count.set(fc);
                let elapsed = last_fps_instant.get().elapsed().as_secs_f64();
                if elapsed >= 1.0 {
                    fps.set((fc as f64 / elapsed) as u32);
                    frame_count.set(0);
                    last_fps_instant.set(Instant::now());
                }

                let is_spinning = spinning.get();
                let spd = speed.get() as f32;

                let mut s = state.lock().unwrap();
                if is_spinning {
                    s.angle_y += 0.02 * spd;
                }
                let angle_x = s.angle_x;
                let angle_y = s.angle_y;
                let zoom = s.zoom;
                drop(s);

                let wf = w as f32;
                let hf = h as f32;
                let w_i = w as i32;
                let h_i = h as i32;
                let npixels = (w * h) as usize;
                let mut buf = vec![0u8; npixels * 4];
                let mut zbuf = vec![f32::INFINITY; npixels];

                for pixel in buf.chunks_exact_mut(4) {
                    pixel[0] = 24;
                    pixel[1] = 24;
                    pixel[2] = 32;
                    pixel[3] = 255;
                }

                let verts: Vec<Vertex> = CUBE_VERTS
                    .iter()
                    .map(|&v| {
                        let v = rotate_x(v, angle_x);
                        let v = rotate_y(v, angle_y);
                        project(v, wf, hf, zoom)
                    })
                    .collect();

                for &(ref indices, color) in &CUBE_FACES {
                    let quad = [
                        verts[indices[0]],
                        verts[indices[1]],
                        verts[indices[2]],
                        verts[indices[3]],
                    ];
                    fill_quad_zbuf(&mut buf, &mut zbuf, w_i, h_i, quad, color);
                }

                writer.submit_frame(&buf, w, h);
            });
        }
    }

    // ── Animated Gradient Demo ───────────────────────────────────────────

    let gradient_surface = create_render_surface();
    {
        let frame = Rc::new(Cell::new(0u64));
        gradient_surface.set_render_callback(move |writer, w, h| {
            let f = frame.get();
            frame.set(f + 1);
            let t = f as f32 * 0.02;

            let size = (w * h * 4) as usize;
            let mut buf = vec![0u8; size];

            for y in 0..h {
                for x in 0..w {
                    let u = x as f32 / w as f32;
                    let v = y as f32 / h as f32;
                    let r = ((u + t).sin() * 0.5 + 0.5) * 255.0;
                    let g = ((v + t * 0.7).cos() * 0.5 + 0.5) * 255.0;
                    let b = (((u + v) * 2.0 + t * 1.3).sin() * 0.5 + 0.5) * 255.0;
                    let idx = ((y * w + x) * 4) as usize;
                    buf[idx] = r as u8;
                    buf[idx + 1] = g as u8;
                    buf[idx + 2] = b as u8;
                    buf[idx + 3] = 255;
                }
            }

            writer.submit_frame(&buf, w, h);
        });
    }

    // ── Layout ───────────────────────────────────────────────────────────

    #[cfg(target_arch = "wasm32")]
    let cube_description = "Thousands of cubes animated by layered sine waves, rendered in a single GPU draw call via wgpu instancing. Each cube rotates independently and is colored by wave height. Drag to orbit, scroll to zoom.";
    #[cfg(not(target_arch = "wasm32"))]
    let cube_description = "A solid 3D cube with a different color per face, rendered via software rasterization. Drag to rotate, scroll to zoom. The overlay panel uses rinch components positioned over the surface.";

    #[cfg(target_arch = "wasm32")]
    let cube_title = "GPU Cube Wave";
    #[cfg(not(target_arch = "wasm32"))]
    let cube_title = "Spinning Cube";

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "RenderSurface" }
                Text { size: "lg", color: "dimmed",
                    "Embed custom renderers into the rinch layout. RenderSurface accepts raw RGBA pixels from any source — game engines, terminal emulators, video decoders, or software rasterizers. Mouse and keyboard events are forwarded to the surface."
                }
            }
            Space { h: "xl" }

            // ── Cube Demo ─────────────────────────────────────────────
            Title { order: 2, {cube_title} }
            Text { size: "sm", color: "dimmed",
                {cube_description}
            }
            Space { h: "sm" }

            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",

                div { style: "position: relative; width: 100%; height: 400px;",
                    RenderSurface { surface: Some(cube_surface), style: "width: 100%; height: 100%;" }

                    // FPS counter
                    div { style: "position: absolute; bottom: 12px; left: 12px;",
                        Paper { p: "4px 10px", radius: "sm",
                            style: "background: rgba(0, 0, 0, 0.7); color: white;",
                            Text { size: "xs", weight: "700", color: "white",
                                {|| format!("{} FPS", fps.get())}
                            }
                        }
                    }

                    // Overlay controls
                    div { style: "position: absolute; top: 12px; right: 12px; width: 200px;",
                        Paper { p: "sm", radius: "md",
                            style: "background: rgba(0, 0, 0, 0.7); color: white;",

                            Stack { gap: "xs",
                                Text { size: "xs", weight: "700", color: "white", "Controls" }

                                Switch {
                                    label: "Animate",
                                    checked_fn: move || spinning.get(),
                                    onchange: move || spinning.update(|v| *v = !*v)
                                }

                                Slider {
                                    label: "Speed",
                                    min: 0.1,
                                    max: 5.0,
                                    step: 0.1,
                                    value_signal: Some(speed),
                                    onchange: move |v: f64| speed.set(v)
                                }

                                Slider {
                                    label: "Grid Size",
                                    min: 5.0,
                                    max: 100.0,
                                    step: 1.0,
                                    value_signal: Some(grid_size_f64),
                                    onchange: move |v: f64| {
                                        grid_size.set(v as u32);
                                        grid_size_f64.set(v);
                                    }
                                }

                                Text { size: "xs", color: "dimmed",
                                    {|| {
                                        let gs = grid_size.get();
                                        format!("{} cubes ({gs} x {gs})", gs * gs)
                                    }}
                                }

                                Button {
                                    variant: "subtle",
                                    size: "xs",
                                    full_width: true,
                                    onclick: move || {
                                        let mut s = cube_state.lock().unwrap();
                                        s.zoom = 1.0;
                                        #[cfg(target_arch = "wasm32")]
                                        { s.angle_x = 0.8; }
                                        #[cfg(not(target_arch = "wasm32"))]
                                        { s.angle_x = 0.4; }
                                        s.angle_y = 0.0;
                                        grid_size.set(50);
                                        grid_size_f64.set(50.0);
                                    },
                                    "Reset View"
                                }
                            }
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ── Animated Gradient ────────────────────────────────────
            Title { order: 2, "Animated Gradient" }
            Text { size: "sm", color: "dimmed",
                "A minimal example showing the render callback API. Each frame computes an animated color gradient and submits it as RGBA pixels."
            }
            Space { h: "sm" }

            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",
                div { style: "width: 100%; height: 200px;",
                    RenderSurface { surface: Some(gradient_surface), style: "width: 100%; height: 100%;" }
                }
            }

            Space { h: "lg" }

            // ── Code example ─────────────────────────────────────────
            Title { order: 2, "Usage" }
            Code {
                "let surface = create_render_surface();\n\nsurface.set_render_callback(|writer, w, h| {\n    let mut pixels = vec![0u8; (w * h * 4) as usize];\n    // ... fill pixels ...\n    writer.submit_frame(&pixels, w, h);\n});\n\nsurface.set_event_handler(|event| match event {\n    SurfaceEvent::MouseDown { x, y, .. } => { /* handle */ }\n    _ => {}\n});\n\nrsx! {\n    RenderSurface { surface: Some(surface) }\n}"
            }
        }
    }
}
