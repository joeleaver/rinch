//! The split-layer compositor (#1865, `docs/design/split-layer-gpu-surfaces.md`).
//!
//! A window with split GPU surfaces is painted as one window-sized **below**
//! slice (Vello's UI texture, straight alpha), and for each surface its
//! texture plus up to two **above** slices the size of the surface's region
//! R: what is painted over the surface inside the clips open around it
//! (`above_in`, rendered **without** those clips) and what is painted after
//! they close (`above_out`). This module puts them together:
//!
//! 1. `fs_below` writes below, premultiplied, everywhere but the regions.
//! 2. `fs_surface`, scissored to R, writes for each pixel of R
//!    ```text
//!    X = above_in ∘ (surface · own ∘ below)
//!    Y = mix(below, X, ancestors)
//!    out = above_out ∘ Y
//!    ```
//!    where `∘` is premultiplied source-over, `own` the surface's own coverage
//!    (image rect, own radius, `object-fit` clip) and `ancestors` the
//!    coverage of the clips open around it. Applying the ancestors' coverage
//!    **once**, to the surface and what is over it together, is what the
//!    in-scene render does (they share the ancestors' clip layer); applying it
//!    to each separately would conflate on an antialiased ring (the #1887
//!    review). What is left is D8's halo, `c(1 − c)(1 − k)·|I − O|`: `I`
//!    everything painted inside the ancestors' clips beneath the surface,
//!    over `O`, what lies behind the clips; `k` how much of what is beneath
//!    the surface shows through it and `above_in`. It is zero when nothing is
//!    painted inside the clips beneath the surface: a card's own background
//!    lies behind its clip (rinch-dom paints it before it opens the clip), so
//!    the common card is exact.
//!
//! Every blend is done **in the shader, in f32, from texel loads**, with no
//! blend state, and each pixel of the target is written once. The surface's
//! sample is computed exactly as Vello's fine shader computes an image fill's
//! (`fine.wgsl`, `CMD_IMAGE`, medium quality: the inverse transform in f32 as
//! `transform_inverse` computes it, the pixel's top-left corner mapped, the
//! bilinear quad clamped to the image, `mix` in the same order, then
//! `rgba · (1 − fg.a) + fg`). Where the in-scene render does the same
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
    // The surface's own coverage when no own mask is bound: 1 inside
    // (x0, y0, x1, y1), on whole device pixels, else 0.
    own_rect: vec4<f32>,
    // The ancestors' coverage when no ancestor mask is bound, likewise.
    ancestors_rect: vec4<f32>,
    // R's origin in device pixels (where the masks and above slices start).
    region_origin: vec4<f32>,
    // (straight alpha, own mask bound, ancestor mask bound, unused)
    flags: vec4<f32>,
    // (above_in bound, above_out bound, unused, unused)
    above: vec4<f32>,
};

@group(0) @binding(0) var below_tex: texture_2d<f32>;
@group(0) @binding(1) var surface_tex: texture_2d<f32>;
@group(0) @binding(2) var own_mask: texture_2d<f32>;
@group(0) @binding(3) var ancestors_mask: texture_2d<f32>;
@group(0) @binding(4) var above_in: texture_2d<f32>;
@group(0) @binding(5) var above_out: texture_2d<f32>;
@group(0) @binding(6) var<uniform> u: Uniforms;

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn in_rect(p: vec2<f32>, r: vec4<f32>) -> bool {
    return p.x >= r.x && p.x < r.z && p.y >= r.y && p.y < r.w;
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
    let below = premul(textureLoad(below_tex, px, 0));
    var rgba = below;

    // The pixel's top-left corner, as Vello 0.7's fine shader mapped it.
    // (Vello 0.9+ maps the centre; `device_from_image` carries the half-pixel
    // shift that makes this corner sample the centre, so the sample points
    // match the in-scene draw's — #1955.)
    let my_xy = floor(in.position.xy);
    let local = px - vec2<i32>(u.region_origin.xy);
    var area = 0.0;
    if u.flags.y > 0.5 {
        area = textureLoad(own_mask, local, 0).a;
    } else if in_rect(my_xy, u.own_rect) {
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
    if u.above.x > 0.5 {
        let over = premul(textureLoad(above_in, local, 0));
        rgba = rgba * (1.0 - over.a) + over;
    }

    // The ancestors' clips, once, over the surface and what is over it.
    var clip = 0.0;
    if u.flags.z > 0.5 {
        clip = textureLoad(ancestors_mask, local, 0).a;
    } else if in_rect(my_xy, u.ancestors_rect) {
        clip = 1.0;
    }
    if clip != 1.0 {
        rgba = mix(below, rgba, clip);
    }

    if u.above.y > 0.5 {
        let over = premul(textureLoad(above_out, local, 0));
        rgba = rgba * (1.0 - over.a) + over;
    }
    return rgba;
}
"#;

/// How much of each pixel of R something covers.
pub(crate) enum Coverage<'a> {
    /// All of each pixel inside `[x0, y0, x1, y1]` (device pixels), none
    /// outside: every edge involved is a rect edge on whole device pixels.
    Rect([u32; 4]),
    /// Vello's coverage, rendered into an R-sized texture (its alpha).
    Mask(&'a TextureView),
}

/// One split surface to composite.
pub(crate) struct SurfaceDraw<'a> {
    /// The surface's texture: `Rgba8Unorm`, the size `extents` says. A
    /// `RenderSurface` texture carries an `Rgba8UnormSrgb` view format; the
    /// view handed here must be the plain `Rgba8Unorm` one, or the texels
    /// are decoded from sRGB on load and no longer match Vello's.
    pub texture: &'a TextureView,
    pub extents: (u32, u32),
    /// Whether the texture holds straight alpha (Vello's `ImageAlphaType::Alpha`).
    pub straight_alpha: bool,
    /// The image-to-device transform with `centre_sampling`'s half-pixel
    /// shift: `fs_surface` maps each pixel's corner through its inverse, which
    /// lands on the image point under the pixel's centre, where Vello samples
    /// the in-scene draw (#1027, #1955). Kurbo coefficients `[a, b, c, d, e, f]`.
    pub device_from_image: [f64; 6],
    /// The surface's own coverage: its image rect under its own clips (inner
    /// radius, `object-fit` clip), **not** the ancestors'.
    pub own: Coverage<'a>,
    /// The coverage of the clips open around the surface.
    pub ancestors: Coverage<'a>,
    /// R: `(x, y, width, height)` in device pixels, inside the target.
    pub region: (u32, u32, u32, u32),
    /// What is painted over the surface while its ancestors' clips are open,
    /// rendered without them (R-sized, straight alpha), if anything.
    pub above_in: Option<&'a TextureView>,
    /// What is painted over it after they close, if anything.
    pub above_out: Option<&'a TextureView>,
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
    own_rect: [f32; 4],
    ancestors_rect: [f32; 4],
    region_origin: [f32; 4],
    flags: [f32; 4],
    above: [f32; 4],
}

/// `outer` minus every rect in `holes`, as disjoint rects (`(x, y, w, h)`).
fn subtract(
    outer: (u32, u32, u32, u32),
    holes: &[(u32, u32, u32, u32)],
) -> Vec<(u32, u32, u32, u32)> {
    let mut rects = vec![outer];
    for &(hx, hy, hw, hh) in holes {
        if hw == 0 || hh == 0 {
            continue;
        }
        let (hx1, hy1) = (hx + hw, hy + hh);
        let mut next = Vec::new();
        for (x, y, w, h) in rects {
            let (x1, y1) = (x + w, y + h);
            if hx >= x1 || hx1 <= x || hy >= y1 || hy1 <= y {
                next.push((x, y, w, h));
                continue;
            }
            // Above, below, left, right of the hole, within this rect.
            if hy > y {
                next.push((x, y, w, hy - y));
            }
            if hy1 < y1 {
                next.push((x, hy1, w, y1 - hy1));
            }
            let (my0, my1) = (hy.max(y), hy1.min(y1));
            if hx > x {
                next.push((x, my0, hx - x, my1 - my0));
            }
            if hx1 < x1 {
                next.push((hx1, my0, x1 - hx1, my1 - my0));
            }
        }
        rects = next;
    }
    rects
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
                texture(4),
                texture(5),
                BindGroupLayoutEntry {
                    binding: 6,
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
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
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
                multiview_mask: None,
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

    /// Composite `below` (straight alpha, `size`) and `draws` into `target`
    /// (`size`), which ends up premultiplied, as
    /// [`super::compositor::LayerCompositor`]'s target does. Draws overlap
    /// nothing (D7), so their order does not matter. Each draw's region is
    /// clamped to the target.
    pub(crate) fn composite(
        &self,
        device: &Device,
        encoder: &mut CommandEncoder,
        below: &TextureView,
        target: &TextureView,
        size: (u32, u32),
        draws: &[SurfaceDraw<'_>],
    ) {
        let clamp = |(x, y, w, h): (u32, u32, u32, u32)| {
            let (x, y) = (x.min(size.0), y.min(size.1));
            (x, y, w.min(size.0 - x), h.min(size.1 - y))
        };
        let regions: Vec<_> = draws.iter().map(|d| clamp(d.region)).collect();
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
            multiview_mask: None,
        });
        // Below, around the regions: each pixel is written once.
        pass.set_pipeline(&self.below);
        pass.set_bind_group(0, &groups[0], &[]);
        for (x, y, w, h) in subtract((0, 0, size.0, size.1), &regions) {
            pass.set_scissor_rect(x, y, w, h);
            pass.draw(0..3, 0..1);
        }
        pass.set_pipeline(&self.surface);
        for (&(x, y, w, h), group) in regions.iter().zip(&groups[1..]) {
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
        let rect = |c: &Coverage<'_>| match c {
            Coverage::Rect(r) => (r.map(|v| v as f32), 0.0),
            Coverage::Mask(_) => ([0.0; 4], 1.0),
        };
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let uniforms = match draw {
            None => Uniforms::zeroed(),
            Some(d) => {
                let m = d.device_from_image;
                let (own_rect, own_mask) = rect(&d.own);
                let (ancestors_rect, ancestors_mask) = rect(&d.ancestors);
                Uniforms {
                    // As Vello encodes it: the f64 transform cast to f32.
                    matrx: [m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32],
                    translate_extents: [
                        m[4] as f32,
                        m[5] as f32,
                        d.extents.0 as f32,
                        d.extents.1 as f32,
                    ],
                    own_rect,
                    ancestors_rect,
                    region_origin: [d.region.0 as f32, d.region.1 as f32, 0.0, 0.0],
                    flags: [flag(d.straight_alpha), own_mask, ancestors_mask, 0.0],
                    above: [
                        flag(d.above_in.is_some()),
                        flag(d.above_out.is_some()),
                        0.0,
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
        let mask = |c: Option<&Coverage<'_>>| match c {
            Some(Coverage::Mask(m)) => view(Some(*m)),
            _ => view(None),
        };
        let views = [
            view(draw.map(|d| d.texture)),
            mask(draw.map(|d| &d.own)),
            mask(draw.map(|d| &d.ancestors)),
            view(draw.and_then(|d| d.above_in)),
            view(draw.and_then(|d| d.above_out)),
        ];
        let mut entries = vec![BindGroupEntry {
            binding: 0,
            resource: BindingResource::TextureView(below),
        }];
        for (i, v) in views.iter().enumerate() {
            entries.push(BindGroupEntry {
                binding: i as u32 + 1,
                resource: BindingResource::TextureView(v),
            });
        }
        entries.push(BindGroupEntry {
            binding: 6,
            resource: buffer.as_entire_binding(),
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("split compositor"),
            layout: &self.layout,
            entries: &entries,
        })
    }
}

use bytemuck::Zeroable as _;

#[cfg(test)]
mod subtract_tests {
    use super::subtract;

    fn area(rects: &[(u32, u32, u32, u32)]) -> u32 {
        rects.iter().map(|r| r.2 * r.3).sum()
    }

    /// Every pixel of `window` outside `holes` covered once, none inside.
    fn assert_once(window: (u32, u32, u32, u32), holes: &[(u32, u32, u32, u32)]) {
        let rects = subtract(window, holes);
        for y in 0..window.3 {
            for x in 0..window.2 {
                let n = rects
                    .iter()
                    .filter(|r| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3)
                    .count();
                let hole = holes
                    .iter()
                    .any(|h| x >= h.0 && x < h.0 + h.2 && y >= h.1 && y < h.1 + h.3);
                assert_eq!(n, usize::from(!hole), "({x}, {y}) with holes {holes:?}");
            }
        }
    }

    /// Regions side by side, and a second region spanning several of the
    /// pieces the first left: each piece's left and right spans are clamped
    /// to its own rows, or the rows above and below are written twice.
    #[test]
    fn regions_spanning_the_pieces_of_others_leave_each_pixel_once() {
        let window = (0, 0, 96, 64);
        assert_once(window, &[(10, 10, 20, 20), (40, 10, 20, 20)]);
        assert_once(window, &[(10, 10, 20, 20), (40, 0, 20, 40)]);
        assert_once(window, &[(30, 20, 20, 10), (10, 5, 70, 50)]);
    }

    #[test]
    fn below_is_drawn_around_the_regions_once() {
        let window = (0, 0, 96, 64);
        let holes = [(20, 12, 40, 26), (70, 40, 10, 10), (0, 0, 5, 5)];
        let rects = subtract(window, &holes);
        assert_eq!(area(&rects), 96 * 64 - 40 * 26 - 10 * 10 - 5 * 5);
        // Every pixel covered once, none in a hole.
        for y in 0..64 {
            for x in 0..96 {
                let n = rects
                    .iter()
                    .filter(|r| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3)
                    .count();
                let hole = holes
                    .iter()
                    .any(|h| x >= h.0 && x < h.0 + h.2 && y >= h.1 && y < h.1 + h.3);
                assert_eq!(n, usize::from(!hole), "({x}, {y})");
            }
        }
    }
}
