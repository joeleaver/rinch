//! #2082: an embedded element shows a GPU texture the host owns, updated
//! every frame without a tree rebuild.
//!
//! The host registers a texture with the context under a name, an element
//! with `data-host-texture="<name>"` draws it, and these read back the
//! overlay texture to check the element's pixels: after the host draws into
//! the texture, swaps it for another of the same size (no repaint), for one
//! of another size, converts a BGRA or sRGB one, and draws one with straight
//! alpha. They need an adapter, so they are `#[ignore]`d, and `gpu` for
//! `pollster`:
//!     cargo test -p rinch --features gpu --test embed_host_texture_2082 -- --ignored

#![cfg(feature = "gpu")]

use rinch::embed::{
    HostTextureError, HostTextureImages, HostTextureOptions, RinchContext, RinchContextConfig,
    RinchOverlayRenderer, TextureAlpha, TextureRefresh,
};
use rinch::prelude::*;
use rinch_dom::perf::Counter;

const W: u32 = 200;
const H: u32 = 100;

/// A white page with two texture elements: `cam` (64x32 at 20,20) and
/// `none` (40x40 at 120,20, blue background, never registered), and one more
/// `cam` element with `object-fit: contain` and a grey background (64x32 at
/// 20,60) for the size change.
const PAGE: &str = "<div style=\"position: absolute; left: 0; top: 0; width: 200px; \
     height: 100px; background: rgb(255,255,255)\">\
     <div data-host-texture=\"cam\" style=\"position: absolute; left: 20px; top: 20px; \
     width: 64px; height: 32px\"></div>\
     <div data-host-texture=\"none\" style=\"position: absolute; left: 120px; top: 20px; \
     width: 40px; height: 40px; background: rgb(0,0,255)\"></div>\
     <div data-host-texture=\"cam\" style=\"position: absolute; left: 20px; top: 60px; \
     width: 64px; height: 32px; object-fit: contain; background: rgb(128,128,128)\"></div>\
     </div>";

fn context() -> RinchContext {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: W,
            height: H,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_inner_html(PAGE);
            root
        },
    );
    ctx.update(&[]);
    ctx
}

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("a GPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

/// A `w`x`h` texture of `format` and `usage` (plus `COPY_DST`), every texel
/// the four bytes `texel` as stored (left empty for a format of another
/// texel size).
fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (w, h): (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    texel: [u8; 4],
) -> wgpu::Texture {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("host texture"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: usage | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if format.block_copy_size(None) == Some(4) {
        fill(queue, &t, texel);
    }
    t
}

/// What the host's game does each frame: draw into its texture.
fn fill(queue: &wgpu::Queue, t: &wgpu::Texture, texel: [u8; 4]) {
    let (w, h) = (t.width(), t.height());
    let data: Vec<u8> = (0..w * h).flat_map(|_| texel).collect();
    queue.write_texture(
        t.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

/// `texture`'s pixels, row-major.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    let row = (W * 4).div_ceil(256) * 256;
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
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
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

struct Host {
    device: wgpu::Device,
    queue: wgpu::Queue,
    overlay: RinchOverlayRenderer,
    ctx: RinchContext,
}

impl Host {
    fn new() -> Self {
        let (device, queue) = gpu();
        let overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
        Self {
            device,
            queue,
            overlay,
            ctx: context(),
        }
    }

    /// Render a frame; its pixels, and how many scene paints it took.
    fn render(&mut self) -> (Vec<[u8; 4]>, u64) {
        self.ctx.reset_perf();
        let _ = self
            .overlay
            .render(&self.device, &self.queue, &mut self.ctx);
        let paints = self.ctx.perf_total().get(Counter::PaintFrames);
        (
            read_back(&self.device, &self.queue, self.overlay.texture()),
            paints,
        )
    }
}

fn at(px: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    px[(y * W + x) as usize]
}

const DIRECT: wgpu::TextureUsages = wgpu::TextureUsages::COPY_SRC;
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREY: [u8; 4] = [128, 128, 128, 255];

/// The direct route (`Rgba8Unorm` + `COPY_SRC`) through a texture's life: it
/// shows where its elements are and nowhere else; the host draws into it and
/// the next render shows that with no repaint; a same-size swap shows the new
/// texture with no repaint; another size repaints once and is fitted by each
/// element's `object-fit`; `OnChange` copies only when told; unregistering
/// shows the background again.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_host_texture_follows_the_host_frame_by_frame_2082() {
    let mut h = Host::new();
    let (dev, q) = (h.device.clone(), h.queue.clone());
    let red = [255, 0, 0, 255];
    let a = texture(
        &dev,
        &q,
        (64, 32),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        red,
    );
    h.ctx.register_texture("cam", &a).unwrap();

    let (px, paints) = h.render();
    assert_eq!(paints, 1, "the first frame paints");
    assert_eq!(at(&px, 50, 35), red, "the fill element shows it");
    assert_eq!(at(&px, 50, 75), red, "a same-aspect contain element too");
    assert_eq!(at(&px, 10, 10), WHITE, "the page around it");
    assert_eq!(at(&px, 140, 40), BLUE, "an unregistered name: background");

    // The game draws its next frame into the same texture.
    let green = [0, 255, 0, 255];
    fill(&q, &a, green);
    let (px, paints) = h.render();
    assert_eq!(at(&px, 50, 35), green, "the next render shows it");
    assert_eq!(paints, 0, "without repainting the scene");

    // Double buffering: a second texture of the same size takes the name.
    let yellow = [255, 255, 0, 255];
    let b = texture(
        &dev,
        &q,
        (64, 32),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        yellow,
    );
    h.ctx.register_texture("cam", &b).unwrap();
    let (px, paints) = h.render();
    assert_eq!(at(&px, 50, 35), yellow, "the swapped-in texture");
    assert_eq!(paints, 0, "a same-size swap repaints nothing");
    // And back: the first texture again, drawn into meanwhile.
    fill(&q, &a, red);
    h.ctx.register_texture("cam", &a).unwrap();
    assert_eq!(at(&h.render().0, 50, 35), red, "swapped back");

    // A square texture: `fill` stretches it over the box, `contain` fits it
    // in the middle over the element's background.
    let magenta = [255, 0, 255, 255];
    let c = texture(
        &dev,
        &q,
        (16, 16),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        magenta,
    );
    h.ctx.register_texture("cam", &c).unwrap();
    let (px, paints) = h.render();
    assert_eq!(paints, 1, "a new size repaints once");
    assert_eq!(at(&px, 22, 22), magenta, "fill: stretched to the corner");
    assert_eq!(at(&px, 52, 76), magenta, "contain: the middle");
    assert_eq!(at(&px, 24, 76), GREY, "contain: the background beside it");

    // `OnChange`: copied once, then only when the host says so.
    let d = texture(
        &dev,
        &q,
        (64, 32),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        red,
    );
    h.ctx
        .register_texture_with(
            "cam",
            &d,
            HostTextureOptions::new().refresh(TextureRefresh::OnChange),
        )
        .unwrap();
    assert_eq!(at(&h.render().0, 50, 35), red);
    fill(&q, &d, green);
    assert_eq!(at(&h.render().0, 50, 35), red, "not told: the copy it has");
    assert!(h.ctx.texture_changed("cam"));
    assert!(!h.ctx.texture_changed("nothing here"));
    let (px, paints) = h.render();
    assert_eq!(at(&px, 50, 35), green, "told: copied");
    assert_eq!(paints, 0);

    assert!(h.ctx.unregister_texture("cam"));
    assert!(!h.ctx.unregister_texture("cam"));
    let (px, paints) = h.render();
    assert_eq!(paints, 1, "a name gone repaints once");
    assert_eq!(at(&px, 50, 35), WHITE, "gone: the page shows");
    assert_eq!(at(&px, 50, 75), GREY, "gone: the element's background");
    let (_, paints) = h.render();
    assert_eq!(paints, 0, "and then nothing");
}

/// A 64x32 texture of four 32x16 quadrants, each texel the four bytes of
/// its quadrant as stored: `[top-left, top-right, bottom-left, bottom-right]`.
fn quadrants(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    quads: [[u8; 4]; 4],
) -> wgpu::Texture {
    let t = texture(device, queue, (64, 32), format, usage, [0; 4]);
    let data: Vec<u8> = (0..32u32)
        .flat_map(|y| (0..64u32).map(move |x| (x, y)))
        .flat_map(|(x, y)| quads[(y >= 16) as usize * 2 + (x >= 32) as usize])
        .collect();
    queue.write_texture(
        t.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64 * 4),
            rows_per_image: Some(32),
        },
        wgpu::Extent3d {
            width: 64,
            height: 32,
            depth_or_array_layers: 1,
        },
    );
    t
}

/// Where the `cam` fill element (64x32 at 20,20) shows each quadrant: two
/// points inside each, off its centre and edges.
const QUAD_POINTS: [[(u32, u32); 2]; 4] = [
    [(28, 24), (44, 32)],
    [(60, 24), (78, 33)],
    [(26, 41), (47, 48)],
    [(57, 40), (80, 49)],
];

fn check_quadrants(px: &[[u8; 4]], want: [[u8; 4]; 4], tol: i32, what: &str) {
    for (q, points) in QUAD_POINTS.iter().enumerate() {
        for &(x, y) in points {
            let got = at(px, x, y);
            let near = got
                .iter()
                .zip(want[q])
                .all(|(&g, w)| (g as i32 - w as i32).abs() <= tol);
            assert!(
                near,
                "{what}: quadrant {q} at ({x}, {y}) is {got:?}, want {:?}",
                want[q]
            );
        }
    }
}

/// BGRA's bytes in RGBA order.
fn bgra(q: [[u8; 4]; 4]) -> [[u8; 4]; 4] {
    q.map(|[r, g, b, a]| [b, g, r, a])
}

/// The converted routes keep each texel's bytes and its place: `Bgra8Unorm`
/// is swizzled, `Rgba8UnormSrgb` and `Bgra8UnormSrgb` are decoded and encoded
/// again (not left linear), and `Rgba8Unorm` without `COPY_SRC` goes through
/// the pass; four distinct quadrants, so a flipped or mirrored conversion
/// fails. Straight alpha is blended as straight, premultiplied (the
/// default) as premultiplied, quadrant by quadrant.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_host_texture_formats_and_alpha_2082() {
    use wgpu::TextureFormat as F;
    let mut h = Host::new();
    let (dev, q) = (h.device.clone(), h.queue.clone());
    let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
    let quads = [
        [10, 200, 60, 255],
        [230, 40, 90, 255],
        [70, 20, 210, 255],
        [250, 220, 30, 255],
    ];
    // `COPY_SRC` on the BGRA and sRGB ones too: copyable, they are still
    // converted, not copied as they are.
    let both = sampled | DIRECT;
    for (format, usage, stored) in [
        (F::Rgba8Unorm, DIRECT, quads),
        (F::Bgra8Unorm, both, bgra(quads)),
        (F::Bgra8UnormSrgb, both, bgra(quads)),
        (F::Rgba8UnormSrgb, both, quads),
        (F::Rgba8Unorm, sampled, quads),
    ] {
        let t = quadrants(&dev, &q, format, usage, stored);
        h.ctx.register_texture("cam", &t).unwrap();
        check_quadrants(&h.render().0, quads, 1, &format!("{format:?} {usage:?}"));
    }

    // Half-transparent quadrants over the white page. Straight: c * a/255 +
    // 255 * (1 - a/255). Premultiplied: c + 255 * (1 - a/255), saturating.
    let half = [
        [128, 0, 0, 128],
        [0, 128, 0, 128],
        [0, 0, 128, 128],
        [64, 64, 0, 128],
    ];
    let blend = |premultiplied: bool| {
        half.map(|[r, g, b, a]| {
            let k = 1.0 - a as f32 / 255.0;
            let f = |c: u8| {
                let c = if premultiplied {
                    c as f32
                } else {
                    c as f32 * a as f32 / 255.0
                };
                (c + 255.0 * k).round().min(255.0) as u8
            };
            [f(r), f(g), f(b), 255]
        })
    };
    for (format, stored) in [(F::Rgba8Unorm, half), (F::Bgra8Unorm, bgra(half))] {
        let t = quadrants(&dev, &q, format, both, stored);
        h.ctx
            .register_texture_with(
                "cam",
                &t,
                HostTextureOptions::new().alpha(TextureAlpha::Straight),
            )
            .unwrap();
        let (px, paints) = h.render();
        assert_eq!(paints, 1, "another texture route or alpha repaints");
        check_quadrants(&px, blend(false), 2, &format!("{format:?} straight"));
        h.ctx.register_texture("cam", &t).unwrap();
        check_quadrants(
            &h.render().0,
            blend(true),
            2,
            &format!("{format:?} premultiplied"),
        );
    }
}

/// The `HostTexture` component draws the texture under its children, as a
/// canvas or video under positioned overlays: a child box shows over it.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_host_texture_children_draw_over_it_2082() {
    let (device, queue) = gpu();
    let mut overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: W,
            height: H,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |__scope: &mut RenderScope| {
            rsx! {
                div { style: "position: absolute; left: 0; top: 0; width: 200px; height: 100px; background: rgb(255,255,255)",
                    HostTexture { name: "cam", style: "position: absolute; left: 20px; top: 20px; width: 64px; height: 32px",
                        div { style: "position: absolute; left: 8px; top: 8px; width: 16px; height: 16px; background: rgb(0,0,255)" }
                    }
                }
            }
        },
    );
    ctx.update(&[]);
    let red = [255, 0, 0, 255];
    let t = texture(
        &device,
        &queue,
        (64, 32),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        red,
    );
    ctx.register_texture("cam", &t).unwrap();
    let _ = overlay.render(&device, &queue, &mut ctx);
    let px = read_back(&device, &queue, overlay.texture());
    assert_eq!(at(&px, 36, 36), BLUE, "the child, over the texture");
    assert_eq!(at(&px, 70, 30), red, "the texture beside it");
}

/// One overlay renderer drawing two contexts in turn (#1976's turn-taking)
/// keeps each context's registrations: no texture is registered (or
/// converted target allocated) again on a later turn, and each context
/// shows its own texture. And one context rendered through
/// `scene_with_textures` by two Vello renderers in turn draws its texture
/// with each: the scene is painted again for the renderer whose images it
/// names.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_host_textures_across_contexts_and_renderers_2082() {
    let mut h = Host::new();
    let (dev, q) = (h.device.clone(), h.queue.clone());
    let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
    let (red, green) = ([255, 0, 0, 255], [0, 255, 0, 255]);
    // A converted (BGRA) texture in each, of different sizes, so the two
    // contexts cannot share one registration: a re-registration allocates a
    // converted texture.
    let a = texture(
        &dev,
        &q,
        (64, 32),
        wgpu::TextureFormat::Bgra8Unorm,
        sampled,
        [0, 0, 255, 255],
    );
    let b = texture(
        &dev,
        &q,
        (32, 16),
        wgpu::TextureFormat::Bgra8Unorm,
        sampled,
        [0, 255, 0, 255],
    );
    let mut other = context();
    h.ctx.register_texture("cam", &a).unwrap();
    other.register_texture("cam", &b).unwrap();
    let turn = |ctx: &mut RinchContext, overlay: &mut RinchOverlayRenderer| {
        let _ = overlay.render(&dev, &q, ctx);
        at(&read_back(&dev, &q, overlay.texture()), 50, 35)
    };
    assert_eq!(turn(&mut h.ctx, &mut h.overlay), red);
    assert_eq!(turn(&mut other, &mut h.overlay), green);
    let registered = h.overlay.host_texture_registrations();
    assert_eq!(registered, 2, "one per context");
    for _ in 0..3 {
        assert_eq!(turn(&mut h.ctx, &mut h.overlay), red);
        assert_eq!(turn(&mut other, &mut h.overlay), green);
    }
    assert_eq!(
        h.overlay.host_texture_registrations(),
        registered,
        "taking turns registers nothing again"
    );
    // A context dropped: its registrations go at the renderer's next render.
    drop(other);
    assert_eq!(turn(&mut h.ctx, &mut h.overlay), red);

    // Two Vello renderers taking turns on one context's scene.
    let new_vello = || {
        vello::Renderer::new(
            &dev,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("vello")
    };
    let mut ctx = context();
    let c = texture(
        &dev,
        &q,
        (64, 32),
        wgpu::TextureFormat::Rgba8Unorm,
        DIRECT,
        red,
    );
    ctx.register_texture("cam", &c).unwrap();
    // A target cleared to transparent black before each render, so a
    // render that draws nothing there cannot pass on the last one's pixels.
    let target = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("host target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let params = vello::RenderParams {
        base_color: vello::peniko::Color::TRANSPARENT,
        width: W,
        height: H,
        antialiasing_method: rinch::gpu_antialiasing().aa_config(),
    };
    let draw =
        |ctx: &mut RinchContext, vello: &mut vello::Renderer, images: &mut HostTextureImages| {
            fill(&q, &target, [0; 4]);
            ctx.reset_perf();
            let scene = ctx.scene_with_textures(&dev, &q, vello, images);
            vello
                .render_to_texture(
                    &dev,
                    &q,
                    scene,
                    &target.create_view(&Default::default()),
                    &params,
                )
                .unwrap();
            (
                at(&read_back(&dev, &q, &target), 50, 35),
                ctx.perf_total().get(Counter::PaintFrames),
            )
        };
    let mut renderers = [
        (new_vello(), HostTextureImages::new()),
        (new_vello(), HostTextureImages::new()),
    ];
    for round in 0..2 {
        for (k, (vello, images)) in renderers.iter_mut().enumerate() {
            let (got, paints) = draw(&mut ctx, vello, images);
            assert_eq!(got, red, "round {round}, renderer {k}");
            assert_eq!(
                paints, 1,
                "round {round}, renderer {k}: painted for this renderer"
            );
        }
    }
    // The same renderer again: the scene it painted is still its own.
    let (vello, images) = &mut renderers[1];
    assert_eq!(draw(&mut ctx, vello, images), (red, 0));
}

/// What is refused, and that a refusal keeps the texture already under the
/// name; and the host-renderer route, `scene_with_textures`, draws the
/// texture where `scene()` draws none.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_host_texture_refusals_and_the_scene_route_2082() {
    use wgpu::TextureFormat as F;
    let mut h = Host::new();
    let (dev, q) = (h.device.clone(), h.queue.clone());
    let red = [255, 0, 0, 255];
    let a = texture(&dev, &q, (64, 32), F::Rgba8Unorm, DIRECT, red);
    h.ctx.register_texture("cam", &a).unwrap();
    let float = texture(&dev, &q, (8, 8), F::Rgba16Float, DIRECT, [0; 4]);
    assert_eq!(
        h.ctx.register_texture("cam", &float),
        Err(HostTextureError::UnsupportedFormat(F::Rgba16Float))
    );
    let bgra_copy_only = texture(&dev, &q, (8, 8), F::Bgra8Unorm, DIRECT, [0; 4]);
    assert_eq!(
        h.ctx.register_texture("cam", &bgra_copy_only),
        Err(HostTextureError::MissingUsage(
            wgpu::TextureUsages::TEXTURE_BINDING
        ))
    );
    let rgba_unusable = texture(
        &dev,
        &q,
        (8, 8),
        F::Rgba8Unorm,
        wgpu::TextureUsages::empty(),
        [0; 4],
    );
    assert_eq!(
        h.ctx.register_texture("cam", &rgba_unusable),
        Err(HostTextureError::MissingUsage(
            wgpu::TextureUsages::COPY_SRC
        ))
    );
    assert_eq!(at(&h.render().0, 50, 35), red, "the registered one stays");

    // The host renders the scene with its own Vello.
    let mut vello = vello::Renderer::new(
        &dev,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("vello");
    let mut images = HostTextureImages::new();
    let target = h.overlay.texture().clone();
    let params = vello::RenderParams {
        base_color: vello::peniko::Color::TRANSPARENT,
        width: W,
        height: H,
        antialiasing_method: rinch::gpu_antialiasing().aa_config(),
    };
    let mut ctx = context();
    ctx.register_texture("cam", &a).unwrap();
    let scene = ctx.scene_with_textures(&dev, &q, &mut vello, &mut images);
    vello
        .render_to_texture(
            &dev,
            &q,
            scene,
            &target.create_view(&Default::default()),
            &params,
        )
        .unwrap();
    assert_eq!(at(&read_back(&dev, &q, &target), 50, 35), red, "drawn");
    let green = [0, 255, 0, 255];
    fill(&q, &a, green);
    let scene = ctx.scene_with_textures(&dev, &q, &mut vello, &mut images);
    vello
        .render_to_texture(
            &dev,
            &q,
            scene,
            &target.create_view(&Default::default()),
            &params,
        )
        .unwrap();
    assert_eq!(
        at(&read_back(&dev, &q, &target), 50, 35),
        green,
        "refreshed"
    );
}
