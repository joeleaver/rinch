//! The split-layer compositor (#1865, `docs/design/split-layer-gpu-surfaces.md`).
//!
//! A window with split GPU surfaces is painted as one window-sized **below**
//! slice (Vello's UI texture, straight alpha), and for each surface its
//! texture plus an optional **above** slice the size of the surface's region
//! R. This module puts them together into the composited target:
//!
//! 1. `fs_below` writes below, premultiplied, over the whole target.
//! 2. `fs_surface`, scissored to R, writes for each pixel of R
//!    `above + (surface + below · (1 − surface.a)) · (1 − above.a)`.
//!
//! Every blend is done **in the shader, in f32, from texel loads**, with the
//! target written once per pixel (no blend state): the surface's sample is
//! computed exactly as Vello's fine shader computes an image fill's
//! (`fine.wgsl`, `CMD_IMAGE`, medium quality: the inverse transform in f32 as
//! `transform_inverse` computes it, the pixel's top-left corner mapped, the
//! bilinear quad clamped to the image, `mix` in the same order, then
//! `rgba · (1 − fg.a) + fg`). So where the in-scene render does the same
//! arithmetic on the same inputs, the split writes the same bits; what
//! differs is only what the slices store in 8 bits that in-scene keeps in
//! float, which is the tolerance D10 names.
use wgpu::util::DeviceExt;
use wgpu::{
    BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    ColorTargetState, ColorWrites, CommandEncoder, Device, FragmentState, MultisampleState,
    PipelineLayoutDescriptor, PrimitiveState, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, TextureFormat, TextureSampleType,
    TextureView, TextureViewDimension, VertexState,
};

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, -y, 0.0, 1.0);
    return out;
}

struct Uniforms {
    // The image-to-device transform Vello is given (kurbo order a, b, c, d),
    // then (e, f) and the image's extents.
    matrx: vec4<f32>,
    translate_extents: vec4<f32>,
    // Coverage when no mask is bound: 1 inside (x0, y0, x1, y1), on whole
    // device pixels, else 0.
    cover: vec4<f32>,
    // R's origin in device pixels (where the mask and above slices start).
    region_origin: vec4<f32>,
    // (straight alpha, mask bound, above bound, unused)
    flags: vec4<f32>,
};

@group(0) @binding(0) var below_tex: texture_2d<f32>;
@group(0) @binding(1) var surface_tex: texture_2d<f32>;
@group(0) @binding(2) var mask_tex: texture_2d<f32>;
@group(0) @binding(3) var above_tex: texture_2d<f32>;
@group(0) @binding(4) var<uniform> u: Uniforms;

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

@fragment
fn fs_below(in: VertexOutput) -> @location(0) vec4<f32> {
    return premul(textureLoad(below_tex, vec2<i32>(in.position.xy), 0));
}

// `transform_inverse` (vello_shaders, shared/transform.wgsl), verbatim.
fn inverse_matrx(m: vec4<f32>) -> vec4<f32> {
    let inv_det = 1.0 / (m.x * m.w - m.y * m.z);
    return inv_det * vec4(m.w, -m.y, -m.z, m.x);
}

@fragment
fn fs_surface(in: VertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy);
    var rgba = premul(textureLoad(below_tex, px, 0));

    // The pixel's top-left corner, as Vello's fine shader maps it.
    let my_xy = floor(in.position.xy);
    var area = 0.0;
    let local = px - vec2<i32>(u.region_origin.xy);
    if u.flags.y > 0.5 {
        area = textureLoad(mask_tex, local, 0).a;
    } else if my_xy.x >= u.cover.x && my_xy.x < u.cover.z && my_xy.y >= u.cover.y && my_xy.y < u.cover.w {
        area = 1.0;
    }
    if area != 0.0 {
        let m = u.matrx;
        let inv = inverse_matrx(m);
        let inv_tr = mat2x2(inv.xy, inv.zw) * -u.translate_extents.xy;
        let extents = u.translate_extents.zw;
        let atlas_max = extents - vec2(1.0);
        var atlas_uv = inv.xy * my_xy.x + inv.zw * my_xy.y + inv_tr;
        atlas_uv.x = clamp(atlas_uv.x, 0.0, extents.x);
        atlas_uv.y = clamp(atlas_uv.y, 0.0, extents.y);
        atlas_uv = atlas_uv - vec2(0.5);
        let atlas_uv_clamped = clamp(atlas_uv, vec2(0.0), atlas_max);
        let uv_quad = vec4(floor(atlas_uv_clamped), ceil(atlas_uv_clamped));
        let uv_frac = fract(atlas_uv);
        var a = textureLoad(surface_tex, vec2<i32>(uv_quad.xy), 0);
        var b = textureLoad(surface_tex, vec2<i32>(uv_quad.xw), 0);
        var c = textureLoad(surface_tex, vec2<i32>(uv_quad.zy), 0);
        var d = textureLoad(surface_tex, vec2<i32>(uv_quad.zw), 0);
        if u.flags.x > 0.5 {
            a = premul(a);
            b = premul(b);
            c = premul(c);
            d = premul(d);
        }
        let fg_rgba = mix(mix(a, b, uv_frac.y), mix(c, d, uv_frac.y), uv_frac.x);
        let fg = fg_rgba * area * 1.0;
        rgba = rgba * (1.0 - fg.a) + fg;
    }

    if u.flags.z > 0.5 {
        let above = premul(textureLoad(above_tex, local, 0));
        rgba = rgba * (1.0 - above.a) + above;
    }
    return rgba;
}
"#;

/// How much of each pixel of R the surface covers.
pub(crate) enum Coverage<'a> {
    /// All of each pixel inside this rect, none outside: the surface's image
    /// rect and every clip on it are rectangles on whole device pixels.
    Rect([u32; 4]),
    /// Vello's coverage of the image rect under the surface's clip chain,
    /// rendered into an R-sized texture (alpha), the mask scene of the design.
    Mask(&'a TextureView),
}

/// One split surface to composite.
pub(crate) struct SurfaceDraw<'a> {
    /// The surface's texture: `Rgba8Unorm`, the size `extents` says.
    pub texture: &'a TextureView,
    pub extents: (u32, u32),
    /// Whether the texture holds straight alpha (Vello's `ImageAlphaType::Alpha`).
    pub straight_alpha: bool,
    /// The image-to-device transform the in-scene fill would hand Vello: the
    /// fill's transform times its brush transform (`centre_sampling`), kurbo
    /// coefficients `[a, b, c, d, e, f]`.
    pub device_from_image: [f64; 6],
    pub coverage: Coverage<'a>,
    /// R: `(x, y, width, height)` in device pixels.
    pub region: (u32, u32, u32, u32),
    /// The above slice, R-sized, straight alpha (Vello's output), if any.
    pub above: Option<&'a TextureView>,
}

pub(crate) struct SplitCompositor {
    layout: BindGroupLayout,
    below: RenderPipeline,
    surface: RenderPipeline,
    dummy: TextureView,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    matrx: [f32; 4],
    translate_extents: [f32; 4],
    cover: [f32; 4],
    region_origin: [f32; 4],
    flags: [f32; 4],
}

impl SplitCompositor {
    /// Pipelines writing `format` (the composited target's: the window
    /// surface's format, `Rgba8Unorm` in tests).
    pub(crate) fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("split compositor"),
            source: ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("split compositor"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                texture(3),
                BindGroupLayoutEntry {
                    binding: 4,
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
            label: Some("split compositor"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("split compositor"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    // No blend state: every blend is in the shader, in f32.
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: None,
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
        };
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("split compositor: unbound"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            below: pipeline("fs_below"),
            surface: pipeline("fs_surface"),
            layout,
            dummy,
        }
    }

    /// Composite `below` (straight alpha, window-sized) and `draws` into
    /// `target` (`size`), which ends up premultiplied, as
    /// [`super::compositor::LayerCompositor`]'s target does. Draws overlap
    /// nothing (D7), so their order does not matter.
    pub(crate) fn composite(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        below: &TextureView,
        target: &TextureView,
        size: (u32, u32),
        draws: &[SurfaceDraw<'_>],
    ) {
        let groups: Vec<_> = std::iter::once(self.bind_group(device, below, None))
            .chain(
                draws
                    .iter()
                    .map(|d| self.bind_group(device, below, Some(d))),
            )
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("split compositor"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.below);
        pass.set_bind_group(0, &groups[0], &[]);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&self.surface);
        for (draw, group) in draws.iter().zip(&groups[1..]) {
            let (x, y, w, h) = draw.region;
            let w = w.min(size.0.saturating_sub(x));
            let h = h.min(size.1.saturating_sub(y));
            if w == 0 || h == 0 {
                continue;
            }
            pass.set_scissor_rect(x, y, w, h);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn bind_group(
        &self,
        device: &Device,
        below: &TextureView,
        draw: Option<&SurfaceDraw<'_>>,
    ) -> wgpu::BindGroup {
        let uniforms = match draw {
            None => Uniforms::zeroed(),
            Some(d) => {
                let m = d.device_from_image;
                let (cover, mask) = match d.coverage {
                    Coverage::Rect(r) => (r.map(|v| v as f32), 0.0),
                    Coverage::Mask(_) => ([0.0; 4], 1.0),
                };
                Uniforms {
                    // As Vello encodes it: the f64 transform cast to f32.
                    matrx: [m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32],
                    translate_extents: [
                        m[4] as f32,
                        m[5] as f32,
                        d.extents.0 as f32,
                        d.extents.1 as f32,
                    ],
                    cover,
                    region_origin: [d.region.0 as f32, d.region.1 as f32, 0.0, 0.0],
                    flags: [
                        if d.straight_alpha { 1.0 } else { 0.0 },
                        mask,
                        if d.above.is_some() { 1.0 } else { 0.0 },
                        0.0,
                    ],
                }
            }
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("split compositor uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = |v: Option<&TextureView>| v.unwrap_or(&self.dummy).clone();
        let surface = view(draw.map(|d| d.texture));
        let mask = view(draw.and_then(|d| match d.coverage {
            Coverage::Mask(m) => Some(m),
            Coverage::Rect(_) => None,
        }));
        let above = view(draw.and_then(|d| d.above));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("split compositor"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(below),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&surface),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&mask),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&above),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
            ],
        })
    }
}

use bytemuck::Zeroable as _;
