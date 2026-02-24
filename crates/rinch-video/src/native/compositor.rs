//! Compositing render pass for video + Vello UI.
//!
//! When video is playing, the simple `copy_texture_to_texture` in the
//! render pipeline is replaced with a compositing pass that:
//! 1. Clears the output with black
//! 2. Blits each video texture into its viewport region (N passes)
//! 3. Alpha-blends the Vello UI texture on top (1 pass)
//!
//! This uses a fullscreen quad with two texture samplers and two
//! separate render pipelines (video blit + UI overlay).

use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendComponent, BlendFactor,
    BlendOperation, BlendState, ColorTargetState, ColorWrites, CommandEncoder, Device,
    FragmentState, MultisampleState, PipelineLayoutDescriptor, PrimitiveState, Queue,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, Texture, TextureFormat, TextureSampleType,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexState,
};

/// WGSL shader with two fragment entry points:
/// - `fs_video`: blits video into a viewport rect, discards outside
/// - `fs_ui`: alpha-blends Vello UI (premultiplied alpha) on top
const VIDEO_BLIT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, -y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, y * 0.5 + 0.5);
    return out;
}

struct Uniforms {
    video_rect: vec4<f32>,
    surface_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var video_tex: texture_2d<f32>;
@group(0) @binding(1) var ui_tex: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;
@group(0) @binding(3) var<uniform> uniforms: Uniforms;

// Blit video into viewport rect, discard pixels outside.
// Multiple video layers can be composited by running this pass
// multiple times with different viewport rects.
@fragment
fn fs_video(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let vr = uniforms.video_rect;

    // Only write inside the video viewport rect
    if uv.x >= vr.x && uv.x <= vr.x + vr.z && uv.y >= vr.y && uv.y <= vr.y + vr.w {
        let video_uv = vec2<f32>(
            (uv.x - vr.x) / vr.z,
            (uv.y - vr.y) / vr.w,
        );
        let video_color = textureSample(video_tex, tex_sampler, video_uv);
        return vec4<f32>(video_color.rgb, 1.0);
    }
    discard;
}

// Alpha-blend UI on top of composited video layers.
// Vello outputs premultiplied alpha, so we use premultiplied blending.
@fragment
fn fs_ui(in: VertexOutput) -> @location(0) vec4<f32> {
    let ui = textureSample(ui_tex, tex_sampler, in.uv);
    return ui;
}
"#;

/// Uniforms for the compositor shader.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    video_rect: [f32; 4],
    surface_size: [f32; 2],
    _pad: [f32; 2],
}

/// GPU pipeline for compositing video frames with the Vello UI.
///
/// Two pipelines share the same bind group layout and vertex shader:
/// - `video_pipeline`: uses `fs_video`, `BlendState::REPLACE` — blits video into viewport rect
/// - `ui_pipeline`: uses `fs_ui`, premultiplied alpha blending — overlays Vello UI on top
pub struct VideoCompositor {
    /// Pipeline for blitting video into a viewport rect.
    video_pipeline: RenderPipeline,
    /// Pipeline for alpha-blending UI on top.
    ui_pipeline: RenderPipeline,
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

impl VideoCompositor {
    /// Create a new compositor for the given surface format.
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("video compositor shader"),
            source: ShaderSource::Wgsl(VIDEO_BLIT_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("video compositor bind group layout"),
            entries: &[
                // Video texture
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // UI texture
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniforms
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("video compositor pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Shared vertex state (both pipelines use the same vertex shader)
        let vertex_state = VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        };

        // Video blit pipeline: opaque write into viewport rect (discard outside)
        let video_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("video blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: vertex_state.clone(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_video"),
                targets: &[Some(ColorTargetState {
                    format: surface_format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // UI overlay pipeline: premultiplied alpha blending
        let ui_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("video UI overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: vertex_state,
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_ui"),
                targets: &[Some(ColorTargetState {
                    format: surface_format,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("video compositor sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            video_pipeline,
            ui_pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Blit a video frame into its viewport region on the target texture.
    ///
    /// Uses `fs_video` which discards pixels outside the viewport rect,
    /// so multiple video layers can be composited by calling this repeatedly.
    ///
    /// `clear`: if true, clear target to black first (use for first layer).
    pub fn blit_video(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        video_texture: &Texture,
        target: &TextureView,
        viewport: (f32, f32, f32, f32),
        surface_size: (u32, u32),
        clear: bool,
    ) {
        let video_view = video_texture.create_view(&TextureViewDescriptor::default());

        // Convert viewport from logical pixels to UV coordinates
        let sw = surface_size.0 as f32;
        let sh = surface_size.1 as f32;
        let uniforms = Uniforms {
            video_rect: [
                viewport.0 / sw,
                viewport.1 / sh,
                viewport.2 / sw,
                viewport.3 / sh,
            ],
            surface_size: [sw, sh],
            _pad: [0.0, 0.0],
        };

        // Each blit pass needs its own uniform buffer because queue.write_buffer()
        // is deferred — the GPU processes ALL staged writes before executing ANY
        // command buffers. If we reuse one buffer, all passes would see the last
        // viewport rect. Creating a per-pass buffer (32 bytes) avoids this.
        let pass_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video blit uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&pass_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // The video blit pass only uses binding 0 (video_tex), but we still
        // need to provide all bindings to satisfy the bind group layout.
        // We use the video texture for the unused ui_tex slot.
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("video blit bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&video_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&video_view), // unused by fs_video
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: pass_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let load_op = if clear {
            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
        } else {
            wgpu::LoadOp::Load
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video blit pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.video_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Alpha-blend the Vello UI texture on top of the composited video layers.
    ///
    /// Uses premultiplied alpha blending (Vello outputs premultiplied alpha).
    /// Call this once after all `blit_video()` passes are done.
    pub fn overlay_ui(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        ui_texture: &Texture,
        target: &TextureView,
    ) {
        let ui_view = ui_texture.create_view(&TextureViewDescriptor::default());

        // UI overlay only uses binding 1 (ui_tex), but we fill all bindings.
        // Create a dummy uniform buffer to satisfy the bind group layout.
        let dummy_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI overlay dummy uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("UI overlay bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&ui_view), // unused by fs_ui
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&ui_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&self.sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: dummy_uniforms.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI overlay pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // preserve video layers underneath
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.ui_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
