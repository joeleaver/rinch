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

/// WGSL shader with these fragment entry points:
/// - `fs_layer`: blits a layer into a viewport rect, discards outside
/// - `fs_ui`: alpha-blends the Vello UI on top. Vello 0.7 stores straight
///   (not premultiplied) alpha (`fine.wgsl` divides by alpha at the final
///   store), so the colour is premultiplied here and blended `One,
///   OneMinusSrcAlpha`: the target then holds premultiplied pixels.
/// - `fs_present_*`: one full-surface `REPLACE` pass converting the
///   alpha encoding on the way to the window surface ([`PresentAlpha`]).
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
    // Border radii in pixels: (top-left, top-right, bottom-right, bottom-left)
    border_radius: vec4<f32>,
    // Clip rect in UV coords: (x, y, w, h). When w > 0, pixels outside are discarded.
    clip_rect: vec4<f32>,
};

@group(0) @binding(0) var layer_tex: texture_2d<f32>;
@group(0) @binding(1) var ui_tex: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;
@group(0) @binding(3) var<uniform> uniforms: Uniforms;

// SDF for a rounded rectangle centered at origin with half-size `b` and
// per-corner radii `r` = (tl, tr, br, bl).
fn sd_rounded_rect(p: vec2<f32>, b: vec2<f32>, r: vec4<f32>) -> f32 {
    // Select radius based on quadrant
    var rx: vec2<f32>;
    if p.x > 0.0 {
        rx = vec2<f32>(r.y, r.z); // right side: tr, br
    } else {
        rx = vec2<f32>(r.x, r.w); // left side: tl, bl
    }
    var radius: f32;
    if p.y > 0.0 {
        radius = rx.y; // bottom
    } else {
        radius = rx.x; // top
    }
    let q = abs(p) - b + vec2<f32>(radius, radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - radius;
}

// Blit layer into viewport rect, discard pixels outside.
// Multiple layers can be composited by running this pass
// multiple times with different viewport rects.
@fragment
fn fs_layer(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let vr = uniforms.layer_rect;

    // Clip to overflow ancestor bounds when set (clip_rect.z > 0 means active)
    let cr = uniforms.clip_rect;
    if cr.z > 0.0 {
        if uv.x < cr.x || uv.x > cr.x + cr.z || uv.y < cr.y || uv.y > cr.y + cr.w {
            discard;
        }
    }

    // Only write inside the layer viewport rect
    if uv.x >= vr.x && uv.x <= vr.x + vr.z && uv.y >= vr.y && uv.y <= vr.y + vr.w {
        // Rounded corner clipping
        let r = uniforms.border_radius;
        if r.x > 0.0 || r.y > 0.0 || r.z > 0.0 || r.w > 0.0 {
            // Convert UV to pixel coordinates relative to viewport center
            let pixel = uv * uniforms.surface_size;
            let center = vec2<f32>(
                (vr.x + vr.z * 0.5) * uniforms.surface_size.x,
                (vr.y + vr.w * 0.5) * uniforms.surface_size.y,
            );
            let half_size = vec2<f32>(
                vr.z * 0.5 * uniforms.surface_size.x,
                vr.w * 0.5 * uniforms.surface_size.y,
            );
            let d = sd_rounded_rect(pixel - center, half_size, r);
            if d > 0.0 {
                discard;
            }
        }

        let layer_uv = vec2<f32>(
            (uv.x - vr.x) / vr.z,
            (uv.y - vr.y) / vr.w,
        );
        let layer_color = textureSample(layer_tex, tex_sampler, layer_uv);
        return layer_color;
    }
    discard;
}

// Alpha-blend UI on top of composited layers. Vello stores straight
// alpha: premultiply, then blend One / OneMinusSrcAlpha.
@fragment
fn fs_ui(in: VertexOutput) -> @location(0) vec4<f32> {
    let ui = textureSample(ui_tex, tex_sampler, in.uv);
    return vec4<f32>(ui.rgb * ui.a, ui.a);
}

// Present passes (REPLACE, full surface): the source as it is, ...
@fragment
fn fs_present_straight(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(ui_tex, tex_sampler, in.uv);
}

// ... straight alpha premultiplied (Vello's output to a PreMultiplied surface), ...
@fragment
fn fs_present_premultiply(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(ui_tex, tex_sampler, in.uv);
    return vec4<f32>(c.rgb * c.a, c.a);
}

// ... or premultiplied alpha made straight (the composited layers to a
// PostMultiplied surface).
@fragment
fn fs_present_unpremultiply(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(ui_tex, tex_sampler, in.uv);
    if c.a <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return vec4<f32>(min(c.rgb / c.a, vec3<f32>(1.0)), c.a);
}
"#;

/// Uniforms for the compositor shader.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    layer_rect: [f32; 4],
    surface_size: [f32; 2],
    _pad: [f32; 2],
    /// Border radii in physical pixels: [top-left, top-right, bottom-right, bottom-left].
    border_radius: [f32; 4],
    /// Clip rect in UV coords: [x, y, w, h]. w > 0 means active.
    clip_rect: [f32; 4],
}

/// How a present pass converts the pixels it copies to the window surface
/// (#1523): the surface's `CompositeAlphaMode` says how the compositor reads
/// them, and Vello's own output is straight alpha while the composited layers
/// are premultiplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentAlpha {
    /// As they are (and in the surface's channel order).
    Straight,
    /// Straight alpha made premultiplied.
    Premultiply,
    /// Premultiplied alpha made straight.
    Unpremultiply,
}

/// GPU pipeline for compositing RGBA layers with the Vello UI.
///
/// Two pipelines share the same bind group layout and vertex shader:
/// - `layer_pipeline`: uses `fs_layer`, `BlendState::REPLACE` — blits layer into viewport rect
/// - `ui_pipeline`: uses `fs_ui`, premultiplied alpha blending — overlays Vello UI on top
/// - `present_pipelines`: `fs_present_*`, `BlendState::REPLACE` — the copy to the surface
pub struct LayerCompositor {
    /// Pipeline for blitting a layer into a viewport rect.
    layer_pipeline: RenderPipeline,
    /// Pipeline for alpha-blending UI on top.
    ui_pipeline: RenderPipeline,
    /// Present passes, by [`PresentAlpha`] (`Straight`, `Premultiply`,
    /// `Unpremultiply`).
    present_pipelines: [RenderPipeline; 3],
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

        // UI overlay pipeline: premultiplied alpha blending (fs_ui premultiplies)
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

        let present_pipelines = [
            "fs_present_straight",
            "fs_present_premultiply",
            "fs_present_unpremultiply",
        ]
        .map(|entry_point| {
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: Some(entry_point),
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
            })
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
            present_pipelines,
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
        border_radius: [f32; 4],
        clip_rect: Option<(f32, f32, f32, f32)>,
    ) {
        let layer_view = layer_texture.create_view(&TextureViewDescriptor::default());

        // Convert viewport from logical pixels to UV coordinates
        let sw = surface_size.0 as f32;
        let sh = surface_size.1 as f32;
        let clip_uv = match clip_rect {
            Some((cx, cy, cw, ch)) => [cx / sw, cy / sh, cw / sw, ch / sh],
            None => [0.0, 0.0, 0.0, 0.0], // w=0 means inactive
        };
        let uniforms = Uniforms {
            layer_rect: [
                viewport.0 / sw,
                viewport.1 / sh,
                viewport.2 / sw,
                viewport.3 / sh,
            ],
            surface_size: [sw, sh],
            _pad: [0.0, 0.0],
            border_radius,
            clip_rect: clip_uv,
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
        border_radius: [f32; 4],
        clip_rect: Option<(f32, f32, f32, f32)>,
    ) {
        let sw = surface_size.0 as f32;
        let sh = surface_size.1 as f32;
        let clip_uv = match clip_rect {
            Some((cx, cy, cw, ch)) => [cx / sw, cy / sh, cw / sw, ch / sh],
            None => [0.0, 0.0, 0.0, 0.0],
        };
        let uniforms = Uniforms {
            layer_rect: [
                viewport.0 / sw,
                viewport.1 / sh,
                viewport.2 / sw,
                viewport.3 / sh,
            ],
            surface_size: [sw, sh],
            _pad: [0.0, 0.0],
            border_radius,
            clip_rect: clip_uv,
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
    /// Vello's straight alpha is premultiplied in the shader and blended
    /// premultiplied, so the target ends up holding premultiplied pixels.
    /// Call this once after all `blit_layer()` passes are done.
    pub fn overlay_ui(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        ui_texture: &Texture,
        target: &TextureView,
    ) {
        self.full_surface_pass(
            device,
            encoder,
            ui_texture,
            target,
            &self.ui_pipeline,
            wgpu::LoadOp::Load, // preserve layers underneath
        );
    }

    /// Copy `source` to the whole of `target` (the window surface),
    /// converting its alpha encoding as `alpha` says. Every pixel is
    /// written, so nothing an earlier frame left in the target survives.
    pub fn present(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        source: &Texture,
        target: &TextureView,
        alpha: PresentAlpha,
    ) {
        let pipeline = &self.present_pipelines[match alpha {
            PresentAlpha::Straight => 0,
            PresentAlpha::Premultiply => 1,
            PresentAlpha::Unpremultiply => 2,
        }];
        self.full_surface_pass(
            device,
            encoder,
            source,
            target,
            pipeline,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        );
    }

    /// A fullscreen draw of `pipeline` sampling `ui_texture` into `target`.
    fn full_surface_pass(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        ui_texture: &Texture,
        target: &TextureView,
        pipeline: &RenderPipeline,
        load: wgpu::LoadOp<wgpu::Color>,
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
                    load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod gpu_alpha_tests {
    //! Real GPU fixtures for the present passes (#1523). They need an
    //! adapter, so they are `#[ignore]`d: run them with
    //! `cargo test -p rinch --features gpu --lib -- --ignored gpu_alpha`.
    use super::*;
    use vello::kurbo::{Affine, Rect};
    use vello::peniko::{Color, Fill};

    const W: u32 = 8;
    const H: u32 = 4;

    fn device() -> (Device, Queue) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a GPU adapter (these fixtures are #[ignore]d for hosts without one)");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device")
    }

    fn texture(device: &Device, format: TextureFormat, usage: wgpu::TextureUsages) -> Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu_alpha_tests"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    /// Vello's straight-alpha output for a frame whose left half is
    /// `rgba(255, 255, 255, 0.5)` and right half unpainted, over a
    /// transparent base: what a transparent window renders.
    fn vello_frame(device: &Device, queue: &Queue) -> Texture {
        let target = texture(
            device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let mut renderer = renderer(device);
        let mut scene = vello::Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 1.0, 1.0, 0.5]),
            None,
            &Rect::new(0.0, 0.0, (W / 2) as f64, H as f64),
        );
        render(device, queue, &mut renderer, &target, &scene);
        target
    }

    fn renderer(device: &Device) -> vello::Renderer {
        vello::Renderer::new(
            device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("vello")
    }

    fn render(
        device: &Device,
        queue: &Queue,
        renderer: &mut vello::Renderer,
        target: &Texture,
        scene: &vello::Scene,
    ) {
        render_over(device, queue, renderer, target, scene, Color::TRANSPARENT);
    }

    fn render_over(
        device: &Device,
        queue: &Queue,
        renderer: &mut vello::Renderer,
        target: &Texture,
        scene: &vello::Scene,
        base_color: Color,
    ) {
        let view = target.create_view(&TextureViewDescriptor::default());
        renderer
            .render_to_texture(
                device,
                queue,
                scene,
                &view,
                &vello::RenderParams {
                    base_color,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("render");
    }

    /// Read `texture` back as RGBA8 rows (no padding: `W * 4` = 32 bytes,
    /// padded to wgpu's 256-byte row alignment).
    fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
        let row = 256u32;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range();
        (0..H)
            .flat_map(|y| {
                let data = &data;
                (0..W).map(move |x| {
                    let i = (y * row + x * 4) as usize;
                    [data[i], data[i + 1], data[i + 2], data[i + 3]]
                })
            })
            .collect()
    }

    fn present(alpha: PresentAlpha) -> Vec<[u8; 4]> {
        let (device, queue) = device();
        let frame = vello_frame(&device, &queue);
        let compositor = LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
        let surface = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = surface.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        compositor.present(&device, &mut encoder, &frame, &view, alpha);
        queue.submit(Some(encoder.finish()));
        read(&device, &queue, &surface)
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
    }

    /// 50% white over a transparent window reaches a `PreMultiplied`
    /// surface as `(128, 128, 128, 128)`, what the software renderer and a
    /// browser composite (over green: `(128, 228, 128)`). The straight copy
    /// it replaces handed over `(255, 255, 255, 128)`, which KWin showed as
    /// opaque white; the unpainted half stays `(0, 0, 0, 0)`.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_alpha_premultiply_on_present() {
        let px = present(PresentAlpha::Premultiply);
        assert!(near(px[0], [128, 128, 128, 128]), "painted: {:?}", px[0]);
        assert_eq!(px[W as usize - 1], [0, 0, 0, 0], "unpainted");
        let straight = present(PresentAlpha::Straight);
        assert!(
            near(straight[0], [255, 255, 255, 128]),
            "Vello's own output is straight: {:?}",
            straight[0]
        );
    }

    /// A UI pixel blended over an opaque green layer comes out as the
    /// browser's `(128, 228, 128)`: `fs_ui` premultiplies Vello's straight
    /// output before the `One, OneMinusSrcAlpha` blend (it added the
    /// straight colour and saturated to `(255, 255, 255)`).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_alpha_ui_over_a_layer() {
        let (device, queue) = device();
        let frame = vello_frame(&device, &queue);
        let compositor = LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
        let target = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = target.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 200.0 / 255.0,
                            b: 0.0,
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
        }
        compositor.overlay_ui(&device, &mut encoder, &frame, &view);
        queue.submit(Some(encoder.finish()));
        let px = read(&device, &queue, &target);
        assert!(near(px[0], [128, 228, 128, 255]), "over green: {:?}", px[0]);
        assert_eq!(px[W as usize - 1], [0, 200, 0, 255], "unpainted: the layer");

        // And the composited (premultiplied) pixels made straight again for
        // a `PostMultiplied` surface.
        let post = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let post_view = post.create_view(&TextureViewDescriptor::default());
        let premultiplied = present_into(
            &device,
            &queue,
            &compositor,
            &frame,
            PresentAlpha::Premultiply,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        compositor.present(
            &device,
            &mut encoder,
            &premultiplied,
            &post_view,
            PresentAlpha::Unpremultiply,
        );
        queue.submit(Some(encoder.finish()));
        let px = read(&device, &queue, &post);
        assert!(near(px[0], [255, 255, 255, 128]), "round trip: {:?}", px[0]);
        assert_eq!(px[W as usize - 1], [0, 0, 0, 0]);
    }

    /// #1774: a zero-copy GPU texture layer in an **opaque** window shows
    /// through the UI's hole. The window's present path, step by step: Vello
    /// renders the UI over the base colour `WgpuRenderer` picks
    /// (`ui_base_color`, here with only a GPU layer), the layer is blitted into
    /// the right half over the window's white base, and the UI is blended on
    /// top. The UI paints blue in column 0 and nothing over the hole. The UI
    /// was rendered over opaque white whenever no *CPU* layer was present, so
    /// the hole came out white whatever the layer held.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_alpha_a_gpu_layer_shows_in_an_opaque_window_1774() {
        let (device, queue) = device();
        let window_base = Color::WHITE; // an opaque window's base
        let usage = wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;

        // The UI: an opaque blue column 0, nothing elsewhere.
        let ui = texture(&device, TextureFormat::Rgba8Unorm, usage);
        let mut scene = vello::Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([0.0, 0.0, 1.0, 1.0]),
            None,
            &Rect::new(0.0, 0.0, 1.0, H as f64),
        );
        let base = crate::shell::desktop::ui_base_color(window_base, false, true);
        render_over(&device, &queue, &mut renderer(&device), &ui, &scene, base);

        // The app's layer: all red.
        let layer = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layer_view = layer.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &layer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }

        let compositor = LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
        let target = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = target.create_view(&TextureViewDescriptor::default());
        compositor.blit_layer_view(
            &device,
            &queue,
            &mut encoder,
            &layer_view,
            &view,
            ((W / 2) as f32, 0.0, (W / 2) as f32, H as f32),
            (W, H),
            Some(wgpu::Color::WHITE),
            [0.0; 4],
            None,
        );
        compositor.overlay_ui(&device, &mut encoder, &ui, &view);
        queue.submit(Some(encoder.finish()));
        let px = read(&device, &queue, &target);
        for y in 0..H as usize {
            let row = &px[y * W as usize..(y + 1) * W as usize];
            assert_eq!(row[0], [0, 0, 255, 255], "row {y}: the UI's own paint");
            assert_eq!(row[2], [255, 255, 255, 255], "row {y}: the window base");
            for (x, p) in row.iter().enumerate().skip((W / 2) as usize) {
                assert_eq!(
                    *p,
                    [255, 0, 0, 255],
                    "row {y} col {x}: the layer, through the hole"
                );
            }
        }
    }

    fn present_into(
        device: &Device,
        queue: &Queue,
        compositor: &LayerCompositor,
        source: &Texture,
        alpha: PresentAlpha,
    ) -> Texture {
        let out = texture(
            device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let view = out.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        compositor.present(device, &mut encoder, source, &view, alpha);
        queue.submit(Some(encoder.finish()));
        out
    }

    /// A 4x2 image of distinct opaque greys.
    const TEXELS: [u8; 8] = [10, 200, 40, 250, 120, 0, 230, 70];

    /// Render `TEXELS` through `VelloPainter::draw_image` at `transform`
    /// and read the target back.
    fn vello_image(transform: Affine) -> Vec<[u8; 4]> {
        use rinch_dom::paint::painter::{PaintImage, Painter};
        let (device, queue) = device();
        let target = texture(
            &device,
            TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let data: Vec<u8> = TEXELS.iter().flat_map(|&g| [g, g, g, 255]).collect();
        let mut painter = rinch_dom::paint::vello_painter::VelloPainter::new();
        painter.draw_image(
            &PaintImage {
                data: &data,
                width: 4,
                height: 2,
                decoded: None,
                opaque: true,
            },
            transform,
        );
        let mut renderer = renderer(&device);
        render(&device, &queue, &mut renderer, &target, painter.scene());
        read(&device, &queue, &target)
    }

    /// #1027: an image at a whole-pixel origin, scale 1, reproduces its
    /// texels as tiny-skia does. Vello 0.7 samples bilinearly from the
    /// pixel's corner, so each pixel read half its left/top neighbour
    /// (texel 200 came out 105) until the painter moved the sample to the
    /// pixel's centre.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_alpha_vello_draw_image_at_a_whole_pixel_origin_is_exact() {
        let px = vello_image(Affine::translate((2.0, 1.0)));
        for iy in 0..2u32 {
            for ix in 0..4u32 {
                let got = px[((1 + iy) * W + 2 + ix) as usize];
                let g = TEXELS[(iy * 4 + ix) as usize];
                assert!(
                    near(got, [g, g, g, 255]),
                    "texel ({ix}, {iy}): {got:?}, want {g}"
                );
            }
        }
    }

    /// #1027 off the fixed point: at scale 2 each pixel is the bilinear
    /// sample at its centre (pixel `x` reads image `(x + 0.5) / 2 - 0.5`).
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_alpha_vello_draw_image_scaled_samples_pixel_centres() {
        let px = vello_image(Affine::scale(2.0));
        let texel = |u: f64, v: f64| {
            let (u, v) = (u.clamp(0.0, 3.0) as usize, v.clamp(0.0, 1.0) as usize);
            TEXELS[v * 4 + u] as f64
        };
        for y in 0..H {
            for x in 0..W {
                let (u, v) = ((x as f64 + 0.5) / 2.0 - 0.5, (y as f64 + 0.5) / 2.0 - 0.5);
                let (u0, v0) = (u.floor(), v.floor());
                let (fu, fv) = (u - u0, v - v0);
                let top = texel(u0, v0) * (1.0 - fu) + texel(u0 + 1.0, v0) * fu;
                let bottom = texel(u0, v0 + 1.0) * (1.0 - fu) + texel(u0 + 1.0, v0 + 1.0) * fu;
                let want = top * (1.0 - fv) + bottom * fv;
                let got = px[(y * W + x) as usize][0] as f64;
                assert!(
                    (got - want).abs() <= 1.5,
                    "pixel ({x}, {y}): {got}, centre sample {want}"
                );
            }
        }
    }
}
