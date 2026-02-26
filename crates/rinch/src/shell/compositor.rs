//! Compositing render pass for layered content + Vello UI.
//!
//! When composite layers are present (video, render surfaces, etc.), the
//! simple `copy_texture_to_texture` in the render pipeline is replaced with
//! a compositing pass that:
//! 1. Clears the output with black
//! 2. Blits each layer texture into its viewport region (N passes)
//! 3. Alpha-blends the Vello UI texture on top (1 pass)
//!
//! This uses a fullscreen quad with two texture samplers and two
//! separate render pipelines (layer blit + UI overlay).

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
/// - `fs_layer`: blits a layer into a viewport rect, discards outside
/// - `fs_ui`: alpha-blends Vello UI (premultiplied alpha) on top
const LAYER_BLIT_SHADER: &str = r#"
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
    layer_rect: vec4<f32>,
    surface_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var layer_tex: texture_2d<f32>;
@group(0) @binding(1) var ui_tex: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;
@group(0) @binding(3) var<uniform> uniforms: Uniforms;

// Blit layer into viewport rect, discard pixels outside.
// Multiple layers can be composited by running this pass
// multiple times with different viewport rects.
@fragment
fn fs_layer(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let vr = uniforms.layer_rect;

    // Only write inside the layer viewport rect
    if uv.x >= vr.x && uv.x <= vr.x + vr.z && uv.y >= vr.y && uv.y <= vr.y + vr.w {
        let layer_uv = vec2<f32>(
            (uv.x - vr.x) / vr.z,
            (uv.y - vr.y) / vr.w,
        );
        let layer_color = textureSample(layer_tex, tex_sampler, layer_uv);
        return vec4<f32>(layer_color.rgb, 1.0);
    }
    discard;
}

// Alpha-blend UI on top of composited layers.
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
    layer_rect: [f32; 4],
    surface_size: [f32; 2],
    _pad: [f32; 2],
}

/// GPU pipeline for compositing RGBA layers with the Vello UI.
///
/// Two pipelines share the same bind group layout and vertex shader:
/// - `layer_pipeline`: uses `fs_layer`, `BlendState::REPLACE` — blits layer into viewport rect
/// - `ui_pipeline`: uses `fs_ui`, premultiplied alpha blending — overlays Vello UI on top
pub struct LayerCompositor {
    /// Pipeline for blitting a layer into a viewport rect.
    layer_pipeline: RenderPipeline,
    /// Pipeline for alpha-blending UI on top.
    ui_pipeline: RenderPipeline,
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
}

impl LayerCompositor {
    /// Create a new compositor for the given surface format.
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("layer compositor shader"),
            source: ShaderSource::Wgsl(LAYER_BLIT_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("layer compositor bind group layout"),
            entries: &[
                // Layer texture
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
            label: Some("layer compositor pipeline layout"),
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

        // Layer blit pipeline: opaque write into viewport rect (discard outside)
        let layer_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("layer blit pipeline"),
            layout: Some(&pipeline_layout),
            vertex: vertex_state.clone(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_layer"),
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
            label: Some("layer UI overlay pipeline"),
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
            label: Some("layer compositor sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            layer_pipeline,
            ui_pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Blit a layer frame into its viewport region on the target texture.
    ///
    /// Uses `fs_layer` which discards pixels outside the viewport rect,
    /// so multiple layers can be composited by calling this repeatedly.
    ///
    /// `clear_color`: if Some, clear target first (use for first layer).
    #[allow(clippy::too_many_arguments)]
    pub fn blit_layer(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        layer_texture: &Texture,
        target: &TextureView,
        viewport: (f32, f32, f32, f32),
        surface_size: (u32, u32),
        clear_color: Option<wgpu::Color>,
    ) {
        let layer_view = layer_texture.create_view(&TextureViewDescriptor::default());

        // Convert viewport from logical pixels to UV coordinates
        let sw = surface_size.0 as f32;
        let sh = surface_size.1 as f32;
        let uniforms = Uniforms {
            layer_rect: [
                viewport.0 / sw,
                viewport.1 / sh,
                viewport.2 / sw,
                viewport.3 / sh,
            ],
            surface_size: [sw, sh],
            _pad: [0.0, 0.0],
        };

        let pass_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer blit uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&pass_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("layer blit bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&layer_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&layer_view), // unused by fs_layer
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

        let load_op = if let Some(color) = clear_color {
            wgpu::LoadOp::Clear(color)
        } else {
            wgpu::LoadOp::Load
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("layer blit pass"),
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

        pass.set_pipeline(&self.layer_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Blit a GPU texture view directly into its viewport region on the target texture.
    ///
    /// Same as [`blit_layer`] but takes a `TextureView` directly instead of a
    /// `Texture` reference — used for zero-copy compositing when the source
    /// already lives on the GPU (e.g., a game engine's offscreen render target).
    #[allow(clippy::too_many_arguments)]
    pub fn blit_layer_view(
        &self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        layer_view: &TextureView,
        target: &TextureView,
        viewport: (f32, f32, f32, f32),
        surface_size: (u32, u32),
        clear_color: Option<wgpu::Color>,
    ) {
        let sw = surface_size.0 as f32;
        let sh = surface_size.1 as f32;
        let uniforms = Uniforms {
            layer_rect: [
                viewport.0 / sw,
                viewport.1 / sh,
                viewport.2 / sw,
                viewport.3 / sh,
            ],
            surface_size: [sw, sh],
            _pad: [0.0, 0.0],
        };

        let pass_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer blit uniforms (gpu)"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&pass_uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("layer blit bind group (gpu)"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(layer_view),
                },
                BindGroupEntry {
                    binding: 1,
                    // Placeholder: binding 1 is the UI texture slot (used by the
                    // overlay pipeline, not the layer blit pipeline). Must be
                    // provided to satisfy the shared bind group layout.
                    resource: BindingResource::TextureView(layer_view),
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

        let load_op = if let Some(color) = clear_color {
            wgpu::LoadOp::Clear(color)
        } else {
            wgpu::LoadOp::Load
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("layer blit pass (gpu)"),
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

        pass.set_pipeline(&self.layer_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Alpha-blend the Vello UI texture on top of the composited layers.
    ///
    /// Uses premultiplied alpha blending (Vello outputs premultiplied alpha).
    /// Call this once after all `blit_layer()` passes are done.
    pub fn overlay_ui(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        ui_texture: &Texture,
        target: &TextureView,
    ) {
        let ui_view = ui_texture.create_view(&TextureViewDescriptor::default());

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
                    load: wgpu::LoadOp::Load, // preserve layers underneath
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
