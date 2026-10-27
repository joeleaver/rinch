//! #1865 PR 1: the split compositor against the in-scene render, on slices
//! built by hand the way the split painter (PR 2) will build them. They need
//! an adapter, so they are `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_split_`.
//!
//! **The rule (D10 and its amendment).** Every pixel is bit-equal to in-scene
//! except, each only where the case's own renders say so ([`allowance`]):
//! - inside R, where an above slice is translucent or antialiased
//!   (0 < alpha < 255): one level;
//! - inside R, where the surface's own coverage is fractional (its mask's
//!   edge): one level;
//! - inside R, where the surface itself is translucent (texel alpha times
//!   coverage strictly between 0 and 1): one level (amendment);
//! - inside R, every pixel the surface covers when it is resampled (not 1:1
//!   at whole pixels): one level (amendment);
//! - outside R, within one Vello tile (16 px) of it, where the content
//!   painted after the surface is fractional, or (amendment) an ancestor
//!   clip's edge is: one level;
//! - a transparent window, inside R or within a tile of it, where the
//!   in-scene pixel's alpha is below 255: one level (inside R below is read
//!   back from 8 bits of straight alpha and blended again; near R the
//!   inverse clip's blend stack holds below's translucent backdrop in 8 bits;
//!   elsewhere the compositor writes below as the present pass does, exact);
//! - inside R, on an ancestor clip's antialiased edge: D8's bound as restated
//!   on 2026-10-25, c(1 − c)(1 − k) × |I − O| in premultiplied RGBA, plus one
//!   level: `I` everything painted inside the ancestors' clips beneath the
//!   surface, over `O`, what lies behind them; `k` how much of what is
//!   beneath the surface shows through it and `above_in`.
//!
//! The classes are computed from renders of the case's own parts, not from a
//! blanket tolerance, so a regression anywhere else fails the test.
use vello::kurbo::{Affine, Circle, Rect, RoundedRect};
use vello::peniko::{Color, Fill, ImageAlphaType};
use vello::{AaConfig, RenderParams, Renderer, Scene};
use wgpu::{Device, Queue, Texture, TextureFormat, TextureUsages};

use super::split_compositor::{Coverage, SplitCompositor, SurfaceDraw};
use super::split_slices_1865_tests::{device, inverse, renderer};

/// The D10 amendment (Joe, 2026-10-23): one level on translucent surface
/// pixels, on resampled surfaces, and on an ancestor clip's antialiased edge
/// within a tile outside R. With `false` the oracle allows only D10's first
/// classes, and exactly the cases that need the amendment fail: the switch
/// that would send those surfaces in-scene instead.
const D10_AMENDMENT: bool = true;

const W: u32 = 96;
const H: u32 = 64;
const TILE: f64 = 16.0;

const TEAL: Color = Color::from_rgba8(0, 128, 128, 160);
const GOLD: Color = Color::from_rgb8(250, 200, 0);
const NAVY: Color = Color::from_rgba8(0, 0, 128, 200);
const SLATE: Color = Color::from_rgb8(60, 70, 90);
const BAR: Color = Color::from_rgba8(0, 0, 0, 140);

/// Paints some content into a scene under the given transform (identity for
/// the window, a translation by −R's origin for an above slice).
type Paint = fn(&mut Scene, Affine);

fn nothing(_: &mut Scene, _: Affine) {}

/// A page: an opaque slate, a translucent wash and a circle, some of it under
/// the surface.
fn page(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        SLATE,
        None,
        &Rect::new(0.0, 0.0, W as f64, H as f64),
    );
    wash(s, t);
}

/// The translucent wash and the circle alone (a transparent window's page).
fn wash(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        TEAL,
        None,
        &Rect::new(0.0, 0.0, 70.0, 40.0),
    );
    s.fill(
        Fill::NonZero,
        t,
        GOLD,
        None,
        &Circle::new((22.0, 18.0), 9.3),
    );
}

/// Opaque slate only.
fn slate(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        SLATE,
        None,
        &Rect::new(0.0, 0.0, W as f64, H as f64),
    );
}

/// A sidebar after the surface in paint order, nowhere near it.
fn sidebar(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        NAVY,
        None,
        &Rect::new(80.0, 0.0, 96.0, 64.0),
    );
}

/// A player's control bar over the bottom of the surface, wider than it, with
/// antialiased rounded buttons and a translucent backdrop.
fn controls(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        BAR,
        None,
        &Rect::new(14.0, 29.5, 78.0, 37.0),
    );
    for i in 0..6 {
        let x = 18.0 + i as f64 * 9.7;
        s.fill(
            Fill::NonZero,
            t,
            Color::WHITE,
            None,
            &RoundedRect::new(x, 31.3, x + 6.2, 35.9, 2.0),
        );
    }
}

/// A tooltip over the surface's top-right corner, painted after the
/// surface's ancestors closed (outside their clips).
fn tooltip(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        Color::from_rgba8(255, 255, 220, 230),
        None,
        &RoundedRect::new(44.4, 8.2, 76.0, 19.6, 3.0),
    );
}

fn texel_rgb(x: u32, y: u32) -> [u8; 4] {
    [
        ((x * 37 + y * 11) % 256) as u8,
        ((x * 5 + y * 29) % 256) as u8,
        ((x * 13 + 7) % 256) as u8,
        255,
    ]
}

/// Straight alpha, varying: some texels clear, some opaque.
fn texel_straight(x: u32, y: u32) -> [u8; 4] {
    let [r, g, b, _] = texel_rgb(x, y);
    [r, g, b, ((x * 23 + y * 41) % 256) as u8]
}

/// A clip open around the surface: an ancestor's overflow clip.
#[derive(Clone, Copy)]
enum Clip {
    Rect(Rect),
    Rounded(RoundedRect),
}

impl Clip {
    fn push(self, s: &mut Scene, t: Affine) {
        match self {
            Clip::Rect(r) => s.push_clip_layer(Fill::NonZero, t, &r),
            Clip::Rounded(r) => s.push_clip_layer(Fill::NonZero, t, &r),
        }
    }
}

struct Case {
    name: &'static str,
    /// R: `(x, y, w, h)`, whole device pixels.
    r: (u32, u32, u32, u32),
    ancestors: Vec<Clip>,
    /// Before the surface, inside its ancestors' clips.
    before: Paint,
    /// The element's own inner-radius clip, as `paint_surface_frame` pushes it.
    own: Option<RoundedRect>,
    /// The image-to-device transform `fit_image` hands `draw_gpu_image`.
    image: Affine,
    extents: (u32, u32),
    texel: fn(u32, u32) -> [u8; 4],
    straight: bool,
    /// After the surface, inside its ancestors' clips.
    after_in: Paint,
    /// After the surface, once its ancestors' clips have closed.
    after_out: Paint,
    /// Whether each above slice must have something (the case is vacuous
    /// otherwise) or must be empty.
    above_in: bool,
    above_out: bool,
    /// The window renders over transparent, not white.
    transparent: bool,
    /// Painted before the first ancestor clip opens (an ancestor's own
    /// background: rinch-dom paints it before it pushes its overflow clip).
    under: Paint,
    /// Painted inside the first ancestor clip, before the others open.
    between: Paint,
}

fn texture(device: &Device, (w, h): (u32, u32), usage: TextureUsages) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split_compositor_1865_tests"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}

fn vello_target(device: &Device, size: (u32, u32)) -> Texture {
    texture(
        device,
        size,
        TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
    )
}

fn render(
    device: &Device,
    queue: &Queue,
    renderer: &mut Renderer,
    scene: &Scene,
    size: (u32, u32),
    base: Color,
) -> Texture {
    let out = vello_target(device, size);
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &out.create_view(&Default::default()),
            &RenderParams {
                base_color: base,
                width: size.0,
                height: size.1,
                antialiasing_method: AaConfig::Msaa16,
            },
        )
        .expect("render");
    out
}

fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.slice(..).get_mapped_range();
    (0..h)
        .flat_map(|y| {
            let data = &data;
            (0..w).map(move |x| {
                let i = (y * row + x * 4) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            })
        })
        .collect()
}

/// `vello_painter::centre_sampling`, the brush transform the in-scene fill
/// carries (#1027), and the product Vello encodes from it.
fn device_from_image(t: Affine) -> [f64; 6] {
    let brush = t.inverse() * Affine::translate((-0.5, -0.5)) * t;
    (t * brush).as_coeffs()
}

/// A pixel past its allowance: `(x, y, split, in-scene, allowance)`.
type Violation = (u32, u32, [u8; 4], [u8; 4], i32);

struct Split {
    composited: Vec<[u8; 4]>,
    /// The above slices (R-sized), each if it has anything.
    above_in: Option<Vec<[u8; 4]>>,
    above_out: Option<Vec<[u8; 4]>>,
    /// Window-sized renders over transparent of: the after-content alone; the
    /// surface alone under all its clips; its image rect under its own clips
    /// (its own coverage); the ancestors' clips filled white.
    after_alone: Vec<[u8; 4]>,
    surface_alone: Vec<[u8; 4]>,
    own_alone: Vec<[u8; 4]>,
    ancestors_alone: Vec<[u8; 4]>,
    /// The surface under its own clips only (no ancestors'): its alpha
    /// times its own coverage, `s` of D8.
    surface_own: Vec<[u8; 4]>,
    /// D8's `I` and `O`, premultiplied: everything painted inside the
    /// ancestors' clips beneath the surface (between them and before it),
    /// unclipped, over `O`; and `O`, what lies behind the clips (painted
    /// before the first opens: an ancestor's own background) over the base.
    inside: Vec<[u8; 4]>,
    behind: Vec<[u8; 4]>,
}

/// How far a split may move window pixel `(x, y)` from in-scene (D10 and its
/// amendment), from the case's own renders. `0` everywhere no class names.
fn allowance(case: &Case, split: &Split, want: [u8; 4], x: u32, y: u32) -> i32 {
    let (rx, ry, rw, rh) = case.r;
    let r = Rect::new(rx as f64, ry as f64, (rx + rw) as f64, (ry + rh) as f64);
    let p = (x as f64 + 0.5, y as f64 + 0.5);
    let fractional = |a: u8| a != 0 && a != 255;
    let i = (y * W + x) as usize;
    // Each class is one more 8-bit rounding than in-scene's, so where several
    // reach a pixel their levels add: the allowance is the count of classes
    // reaching it (plus D8's halo), not the largest.
    let mut allow = 0;
    // A transparent window, where the window is not opaque: inside R below is
    // read back from 8 bits of straight alpha and blended again, where
    // in-scene blends it in float; within a tile of R, the inverse clip's
    // blend stack holds below's translucent backdrop in 8 bits. Elsewhere the
    // compositor writes below as the present pass would: exact.
    if case.transparent && want[3] < 255 && r.inflate(TILE, TILE).contains(p) {
        allow += 1;
    }
    if r.contains(p) {
        let local = ((y - ry) * rw + (x - rx)) as usize;
        let surface = split.surface_alone[i][3];
        let resampled = case.image.as_coeffs()[..4] != [1.0, 0.0, 0.0, 1.0]
            || case.image.as_coeffs()[4..].iter().any(|v| v.fract() != 0.0);
        let above = |a: &Option<Vec<[u8; 4]>>| a.as_ref().is_some_and(|a| fractional(a[local][3]));
        // Each above slice is stored in 8 bits, then blended.
        allow += i32::from(above(&split.above_in)) + i32::from(above(&split.above_out));
        // The own mask is stored in 8 bits.
        allow += i32::from(fractional(split.own_alone[i][3]));
        // The amendment: a translucent or resampled surface pixel.
        if D10_AMENDMENT && (fractional(surface) || (resampled && surface != 0)) {
            allow += 1;
        }
        // D8 (restated 2026-10-25): on an ancestor's ring, c(1 − c)(1 − k)
        // times |I − O| in premultiplied RGBA, k how much of what is beneath
        // the surface shows through it and above_in; plus the mask's level.
        let c = split.ancestors_alone[i][3];
        if fractional(c) {
            let c = c as f64 / 255.0;
            let s = split.surface_own[i][3] as f64 / 255.0;
            let a = split
                .above_in
                .as_ref()
                .map_or(0.0, |px| px[local][3] as f64 / 255.0);
            let k = (1.0 - s) * (1.0 - a);
            let delta = (0..4)
                .map(|ch| (split.inside[i][ch] as i32 - split.behind[i][ch] as i32).abs())
                .max()
                .unwrap() as f64;
            allow += (c * (1.0 - c) * (1.0 - k) * delta).ceil() as i32 + 1;
        }
    } else if r.inflate(TILE, TILE).contains(p) {
        allow += i32::from(fractional(split.after_alone[i][3]));
        allow += i32::from(D10_AMENDMENT && fractional(split.ancestors_alone[i][3]));
    }
    allow
}

/// How a split is put together; each control breaks one piece.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// The above slices are not composited.
    NoAbove,
    /// The surface's coverage rect is one pixel too narrow.
    NarrowCover,
    /// The texture's alpha type is passed the wrong way round.
    WrongAlpha,
    /// The surface replaces below instead of going over it (below is empty
    /// inside R).
    Replace,
    /// Below is painted without the inverse clip: after-content under the
    /// surface.
    NoInverse,
    /// The ancestors' coverage is applied to the surface alone (the
    /// conflation the #1887 review found): the above slice keeps its clips.
    ClipTwice,
}

fn upload(device: &Device, queue: &Queue, size: (u32, u32), px: &[[u8; 4]]) -> Texture {
    let t = texture(
        device,
        size,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
    );
    queue.write_texture(
        t.as_image_copy(),
        &px.concat(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.0 * 4),
            rows_per_image: Some(size.1),
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    t
}

/// `r` under `t`, if its edges land on whole pixels.
fn whole_pixels(t: Affine, r: Rect) -> Option<Rect> {
    let [_, b, c, _, _, _] = t.as_coeffs();
    if b != 0.0 || c != 0.0 {
        return None;
    }
    let m = t.transform_rect_bbox(r);
    [m.x0, m.y0, m.x1, m.y1]
        .iter()
        .all(|v| v.fract() == 0.0)
        .then_some(m)
}

fn rect_u32(r: Rect) -> [u32; 4] {
    let r = if r.width() > 0.0 && r.height() > 0.0 {
        r
    } else {
        Rect::ZERO
    };
    [r.x0 as u32, r.y0 as u32, r.x1 as u32, r.y1 as u32]
}

fn render_read(
    device: &Device,
    queue: &Queue,
    vello: &mut Renderer,
    scene: &Scene,
    base: Color,
) -> Vec<[u8; 4]> {
    read(
        device,
        queue,
        &render(device, queue, vello, scene, (W, H), base),
    )
}

/// An above slice: rendered, and kept only if it has anything.
fn above_slice(
    device: &Device,
    queue: &Queue,
    vello: &mut Renderer,
    scene: &Scene,
    size: (u32, u32),
) -> Option<(Texture, Vec<[u8; 4]>)> {
    let t = render(device, queue, vello, scene, size, Color::TRANSPARENT);
    let px = read(device, queue, &t);
    px.iter().any(|p| p[3] != 0).then_some((t, px))
}

fn run(case: &Case, fault: Fault) -> (Vec<[u8; 4]>, Split) {
    let (device, queue, errors) = device();
    let mut vello = renderer(&device);
    let (rx, ry, rw, rh) = case.r;
    let window = Rect::new(0.0, 0.0, W as f64, H as f64);
    let r = Rect::new(rx as f64, ry as f64, (rx + rw) as f64, (ry + rh) as f64);
    let to_r = Affine::translate((-(rx as f64), -(ry as f64)));
    let id = Affine::IDENTITY;
    let base = if case.transparent {
        Color::TRANSPARENT
    } else {
        Color::WHITE
    };

    // The surface's texture.
    let (ew, eh) = case.extents;
    let texels: Vec<[u8; 4]> = (0..eh)
        .flat_map(|y| (0..ew).map(move |x| (case.texel)(x, y)))
        .collect();
    let surface = upload(&device, &queue, case.extents, &texels);
    let image_rect = Rect::new(0.0, 0.0, ew as f64, eh as f64);

    // In-scene: what #1861 draws — the image through rinch's own painter.
    let mut vello2 = renderer(&device);
    let mut image = vello2.register_texture(surface.clone());
    if !case.straight {
        image.alpha_type = ImageAlphaType::AlphaPremultiplied;
    }
    let mut painter = rinch_dom::paint::vello_painter::VelloPainter::new();
    rinch_dom::paint::painter::Painter::draw_gpu_image(&mut painter, &image, case.image);
    let surface_draw = |s: &mut Scene| {
        if let Some(own) = case.own {
            s.push_clip_layer(Fill::NonZero, id, &own);
        }
        s.append(painter.scene(), None);
        if case.own.is_some() {
            s.pop_layer();
        }
    };
    let mut in_scene = Scene::new();
    (case.under)(&mut in_scene, id);
    for (n, c) in case.ancestors.iter().enumerate() {
        c.push(&mut in_scene, id);
        if n == 0 {
            (case.between)(&mut in_scene, id);
        }
    }
    (case.before)(&mut in_scene, id);
    surface_draw(&mut in_scene);
    (case.after_in)(&mut in_scene, id);
    for _ in &case.ancestors {
        in_scene.pop_layer();
    }
    (case.after_out)(&mut in_scene, id);
    // In-scene as a window presents it to a premultiplied surface: Vello's
    // straight output through the present pass's premultiply (the
    // composited target is premultiplied). For an opaque window the two
    // are the same pixels.
    let in_scene_ui = render(&device, &queue, &mut vello2, &in_scene, (W, H), base);
    let presented = texture(
        &device,
        (W, H),
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    super::compositor::LayerCompositor::new(&device, TextureFormat::Rgba8Unorm).present(
        &device,
        &mut encoder,
        &in_scene_ui,
        &presented.create_view(&Default::default()),
        super::compositor::PresentAlpha::Premultiply,
    );
    queue.submit(Some(encoder.finish()));
    let want = read(&device, &queue, &presented);

    // The class renders: the surface alone under all its clips, its own
    // coverage, the ancestors' coverage, the before-content unclipped.
    let mut alone = Scene::new();
    for c in &case.ancestors {
        c.push(&mut alone, id);
    }
    surface_draw(&mut alone);
    for _ in &case.ancestors {
        alone.pop_layer();
    }
    let surface_alone = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello2,
            &alone,
            (W, H),
            Color::TRANSPARENT,
        ),
    );
    let mut own_only = Scene::new();
    surface_draw(&mut own_only);
    let surface_own = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello2,
            &own_only,
            (W, H),
            Color::TRANSPARENT,
        ),
    );
    vello2.unregister_texture(image);
    let own_scene = |t: Affine| {
        let mut s = Scene::new();
        if let Some(own) = case.own {
            s.push_clip_layer(Fill::NonZero, t, &own);
        }
        s.fill(
            Fill::NonZero,
            t * case.image,
            Color::WHITE,
            None,
            &image_rect,
        );
        if case.own.is_some() {
            s.pop_layer();
        }
        s
    };
    let ancestors_scene = |t: Affine| {
        let mut s = Scene::new();
        for c in &case.ancestors {
            c.push(&mut s, t);
        }
        s.fill(Fill::NonZero, t, Color::WHITE, None, &window);
        for _ in &case.ancestors {
            s.pop_layer();
        }
        s
    };
    let own_alone = render_read(
        &device,
        &queue,
        &mut vello,
        &own_scene(id),
        Color::TRANSPARENT,
    );
    let ancestors_alone = render_read(
        &device,
        &queue,
        &mut vello,
        &ancestors_scene(id),
        Color::TRANSPARENT,
    );
    let premultiplied = |px: Vec<[u8; 4]>| -> Vec<[u8; 4]> {
        px.iter()
            .map(|p| {
                let m = |c: u8| ((c as u32 * p[3] as u32 + 127) / 255) as u8;
                [m(p[0]), m(p[1]), m(p[2]), p[3]]
            })
            .collect()
    };
    let mut behind = Scene::new();
    (case.under)(&mut behind, id);
    let behind = premultiplied(render_read(&device, &queue, &mut vello, &behind, base));
    let mut inside = Scene::new();
    (case.under)(&mut inside, id);
    (case.between)(&mut inside, id);
    (case.before)(&mut inside, id);
    let inside = premultiplied(render_read(&device, &queue, &mut vello, &inside, base));
    let mut after = Scene::new();
    for c in &case.ancestors {
        c.push(&mut after, id);
    }
    (case.after_in)(&mut after, id);
    for _ in &case.ancestors {
        after.pop_layer();
    }
    (case.after_out)(&mut after, id);
    let after_alone = render_read(&device, &queue, &mut vello, &after, Color::TRANSPARENT);

    // Below: after-content kept out of R by the inverse clip, popped and
    // pushed again around each ancestor's pop.
    let not_r = inverse(window, r);
    let inverse_on = fault != Fault::NoInverse;
    let mut below = Scene::new();
    (case.under)(&mut below, id);
    for (n, c) in case.ancestors.iter().enumerate() {
        c.push(&mut below, id);
        if n == 0 {
            (case.between)(&mut below, id);
        }
    }
    (case.before)(&mut below, id);
    if inverse_on {
        below.push_clip_layer(Fill::NonZero, id, &not_r);
    }
    (case.after_in)(&mut below, id);
    for _ in &case.ancestors {
        if inverse_on {
            below.pop_layer();
        }
        below.pop_layer();
        if inverse_on {
            below.push_clip_layer(Fill::NonZero, id, &not_r);
        }
    }
    (case.after_out)(&mut below, id);
    if inverse_on {
        below.pop_layer();
    }
    let mut below_px = render_read(&device, &queue, &mut vello, &below, base);
    if fault == Fault::Replace {
        for y in ry..ry + rh {
            for x in rx..rx + rw {
                below_px[(y * W + x) as usize] = [0; 4];
            }
        }
    }
    let below = upload(&device, &queue, (W, H), &below_px);

    // Above, in R's coordinates: inside the ancestors (rendered without
    // their clips: the compositor applies them once), and after them.
    let mut above_in = Scene::new();
    if fault == Fault::ClipTwice {
        for c in &case.ancestors {
            c.push(&mut above_in, to_r);
        }
    }
    (case.after_in)(&mut above_in, to_r);
    if fault == Fault::ClipTwice {
        for _ in &case.ancestors {
            above_in.pop_layer();
        }
    }
    let above_in = above_slice(&device, &queue, &mut vello, &above_in, (rw, rh));
    let mut above_out = Scene::new();
    (case.after_out)(&mut above_out, to_r);
    let above_out = above_slice(&device, &queue, &mut vello, &above_out, (rw, rh));

    // Coverage: whole-pixel rects where every edge is one, else Vello masks.
    let own_rect = case
        .own
        .is_none()
        .then(|| whole_pixels(case.image, image_rect))
        .flatten();
    let own_mask = own_rect.is_none().then(|| {
        render(
            &device,
            &queue,
            &mut vello,
            &own_scene(to_r),
            (rw, rh),
            Color::TRANSPARENT,
        )
    });
    let mut ancestors_rect = Some(r);
    for c in &case.ancestors {
        ancestors_rect = match (ancestors_rect, c) {
            (Some(k), Clip::Rect(c)) => whole_pixels(id, *c).map(|c| k.intersect(c)),
            _ => None,
        };
    }
    let ancestors_mask = ancestors_rect.is_none().then(|| {
        render(
            &device,
            &queue,
            &mut vello,
            &ancestors_scene(to_r),
            (rw, rh),
            Color::TRANSPARENT,
        )
    });

    let target = texture(
        &device,
        (W, H),
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let compositor = SplitCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let view = |t: &Texture| t.create_view(&Default::default());
    let surface_view = view(&surface);
    let own_view = own_mask.as_ref().map(view);
    let ancestors_view = ancestors_mask.as_ref().map(view);
    let above_in_view = above_in.as_ref().map(|(t, _)| view(t));
    let above_out_view = above_out.as_ref().map(|(t, _)| view(t));
    let own = match (&own_view, own_rect) {
        (Some(m), _) => Coverage::Mask(m),
        (None, Some(c)) => {
            let mut c = rect_u32(c.intersect(r));
            if fault == Fault::NarrowCover {
                c[2] -= 1;
            }
            Coverage::Rect(c)
        }
        (None, None) => unreachable!(),
    };
    let ancestors = match (&ancestors_view, ancestors_rect) {
        (Some(m), _) => Coverage::Mask(m),
        (None, Some(c)) => Coverage::Rect(rect_u32(c)),
        (None, None) => unreachable!(),
    };
    let no_above = fault == Fault::NoAbove;
    let mut encoder = device.create_command_encoder(&Default::default());
    compositor.composite(
        &device,
        &mut encoder,
        &view(&below),
        &view(&target),
        (W, H),
        &[SurfaceDraw {
            texture: &surface_view,
            extents: case.extents,
            straight_alpha: case.straight != (fault == Fault::WrongAlpha),
            device_from_image: device_from_image(case.image),
            own,
            ancestors,
            region: case.r,
            above_in: above_in_view.as_ref().filter(|_| !no_above),
            above_out: above_out_view.as_ref().filter(|_| !no_above),
            base: None,
        }],
    );
    queue.submit(Some(encoder.finish()));
    let composited = read(&device, &queue, &target);
    assert_eq!(
        *errors.lock().unwrap(),
        Vec::<String>::new(),
        "{}",
        case.name
    );
    (
        want,
        Split {
            composited,
            above_in: above_in.map(|(_, px)| px),
            above_out: above_out.map(|(_, px)| px),
            after_alone,
            surface_alone,
            own_alone,
            ancestors_alone,
            surface_own,
            inside,
            behind,
        },
    )
}

/// The pixels breaking D10, and how many pixels have an allowance.
fn violations(case: &Case, fault: Fault) -> (Vec<Violation>, usize, Split) {
    let (want, split) = run(case, fault);
    let mut out = Vec::new();
    let mut allowed = 0;
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let (got, want) = (split.composited[i], want[i]);
            let allow = allowance(case, &split, want, x, y);
            allowed += usize::from(allow > 0);
            let off = (0..4)
                .map(|c| (got[c] as i32 - want[c] as i32).abs())
                .max()
                .unwrap();
            if off > allow {
                out.push((x, y, got, want, allow));
            }
        }
    }
    (out, allowed, split)
}

fn assert_d10(case: &Case) {
    let (wrong, allowed, split) = violations(case, Fault::None);
    assert_eq!(
        (split.above_in.is_some(), split.above_out.is_some()),
        (case.above_in, case.above_out),
        "{}: the above slices are not as the case says",
        case.name
    );
    eprintln!(
        "{}: {allowed} of {} pixels have an allowance, the rest must be exact",
        case.name,
        W * H
    );
    assert!(
        wrong.is_empty(),
        "{}: {} pixels break D10 (x, y, split, in-scene, allowance): {:?}",
        case.name,
        wrong.len(),
        &wrong[..wrong.len().min(8)]
    );
}

fn plain() -> Case {
    Case {
        name: "opaque frame, 1:1, whole pixels, nothing over it",
        r: (20, 12, 40, 26),
        ancestors: vec![],
        before: page,
        own: None,
        image: Affine::translate((20.0, 12.0)),
        extents: (40, 26),
        texel: texel_rgb,
        straight: false,
        after_in: nothing,
        after_out: sidebar,
        above_in: false,
        above_out: false,
        transparent: false,
        under: nothing,
        between: nothing,
    }
}

fn card() -> Vec<Clip> {
    vec![Clip::Rounded(RoundedRect::new(14.3, 6.6, 52.7, 44.2, 10.0))]
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_an_opaque_frame_is_exact() {
    assert_d10(&plain());
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_straight_alpha_frame_over_the_page() {
    assert_d10(&Case {
        name: "straight-alpha frame over a solid and a translucent page",
        texel: texel_straight,
        straight: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_translucent_frame_over_opaque_slate() {
    // The #1887 review: the amendment's translucent class is not conditional
    // on translucent content beneath.
    assert_d10(&Case {
        name: "translucent frame over opaque slate",
        before: slate,
        texel: texel_straight,
        straight: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_premultiplied_translucent_frame() {
    fn premultiplied(x: u32, y: u32) -> [u8; 4] {
        let [r, g, b, a] = texel_straight(x, y);
        let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        [m(r), m(g), m(b), a]
    }
    assert_d10(&Case {
        name: "premultiplied translucent frame",
        texel: premultiplied,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_whole_pixel_scroll_clip_is_exact() {
    assert_d10(&Case {
        name: "scrolled half out of a whole-pixel scroller (top)",
        ancestors: vec![Clip::Rect(Rect::new(8.0, 20.0, 70.0, 60.0))],
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_scroll_clip_cutting_r_right_and_bottom() {
    // Kills a coverage rect taken inclusive on its right or bottom edge.
    assert_d10(&Case {
        name: "a whole-pixel scroller cutting R's right and bottom",
        ancestors: vec![Clip::Rect(Rect::new(8.0, 5.0, 50.0, 30.0))],
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_its_own_radius_is_a_mask() {
    assert_d10(&Case {
        name: "own inner radius",
        own: Some(RoundedRect::new(20.0, 12.0, 60.0, 38.0, 7.0)),
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_rounded_ancestor_clip() {
    assert_d10(&Case {
        name: "rounded, fractional ancestor clip across the frame",
        ancestors: card(),
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_over_the_frame() {
    assert_d10(&Case {
        name: "a control bar straddling R",
        after_in: controls,
        above_in: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_on_a_rounded_card_without_a_background() {
    // The #1887 review's case: the card's ring under the control bar.
    assert_d10(&Case {
        name: "controls on a rounded card, no background",
        ancestors: card(),
        before: nothing,
        after_in: controls,
        above_in: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_on_a_rounded_card_over_the_page() {
    assert_d10(&Case {
        name: "controls on a rounded card, page inside it",
        ancestors: card(),
        after_in: controls,
        above_in: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_inside_a_rounded_card_with_its_own_radius() {
    assert_d10(&Case {
        name: "controls inside a rounded card, own radius too",
        ancestors: vec![Clip::Rounded(RoundedRect::new(12.0, 8.0, 80.0, 56.0, 6.0))],
        own: Some(RoundedRect::new(20.0, 12.0, 60.0, 38.0, 5.0)),
        after_in: controls,
        above_in: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_tooltip_after_the_card() {
    // Painted after the card's clip closed: not under its coverage.
    assert_d10(&Case {
        name: "a tooltip after the card, over its ring",
        ancestors: card(),
        after_in: controls,
        after_out: tooltip,
        above_in: true,
        above_out: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_scaled_frame_samples_as_vello_does() {
    // A 20x13 frame scaled by 2 into the 40x26 content box (a letterboxed
    // video's case), at whole pixels: Vello's bilinear arm.
    assert_d10(&Case {
        name: "frame scaled x2",
        image: Affine::translate((20.0, 12.0)) * Affine::scale(2.0),
        extents: (20, 13),
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_an_unevenly_scaled_frame_samples_as_vello_does() {
    // 30x20 into 40x26: scale 4/3 by 1.3, so every sample is off the texel
    // grid; the image rect still ends on whole pixels.
    assert_d10(&Case {
        name: "frame scaled 4/3 x 1.3",
        image: Affine::translate((20.0, 12.0)) * Affine::scale_non_uniform(40.0 / 30.0, 1.3),
        extents: (30, 20),
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_frame_at_fractional_offsets() {
    // Kills a repeat for the clamp, on either axis: samples past the left
    // or top edge.
    for (name, image, r) in [
        (
            "offset 19.75",
            Affine::translate((19.75, 12.0)),
            (19, 12, 41, 26),
        ),
        (
            "offset 12.25 down",
            Affine::translate((20.0, 12.25)),
            (20, 12, 40, 27),
        ),
        (
            "offset 20.002, 11.998",
            Affine::translate((20.002, 11.998)),
            (20, 11, 41, 27),
        ),
        (
            "negative origin",
            Affine::translate((-5.5, -3.25)),
            (0, 0, 35, 23),
        ),
    ] {
        assert_d10(&Case {
            name,
            image,
            r,
            ..plain()
        });
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_transparent_window() {
    // Kills below written straight instead of premultiplied.
    assert_d10(&Case {
        name: "transparent window",
        before: wash,
        texel: texel_straight,
        straight: true,
        after_in: controls,
        above_in: true,
        transparent: true,
        ..plain()
    });
}

// ── Positive controls: each fault must be seen, outside the tolerance ──────

fn assert_fault_seen(case: &Case, fault: Fault, what: &str) {
    assert!(
        !violations(case, fault).0.is_empty(),
        "control ({what}): no pixel past its allowance; the oracle cannot see it"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_the_oracle_sees_each_fault() {
    let over = Case {
        name: "a control bar straddling R",
        after_in: controls,
        above_in: true,
        ..plain()
    };
    assert_fault_seen(&over, Fault::NoAbove, "above slice skipped");
    assert_fault_seen(
        &plain(),
        Fault::NarrowCover,
        "coverage one pixel too narrow",
    );
    let straight = Case {
        texel: texel_straight,
        straight: true,
        ..plain()
    };
    assert_fault_seen(&straight, Fault::WrongAlpha, "alpha type swapped");
    assert_fault_seen(&straight, Fault::Replace, "replace instead of over");
    let under = Case {
        texel: texel_straight,
        straight: true,
        after_in: controls,
        above_in: true,
        ..plain()
    };
    assert_fault_seen(&under, Fault::NoInverse, "below without the inverse clip");
    let ring = Case {
        ancestors: card(),
        before: nothing,
        after_in: controls,
        above_in: true,
        ..plain()
    };
    assert_fault_seen(&ring, Fault::ClipTwice, "ancestors' coverage applied twice");
}

/// The composite's per-frame cost against the old layer path's (#1865 design
/// record, *Measured cost*): a new surface frame, the UI unchanged, so no
/// Vello. A measurement, not a test:
/// `cargo test -p rinch --features gpu --release --lib split_composite_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement; needs a GPU adapter"]
fn split_composite_cost_probe() {
    use std::time::{Duration, Instant};
    let (device, queue, _errors) = device();
    let layer = super::compositor::LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let split = SplitCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let rt = TextureUsages::RENDER_ATTACHMENT
        | TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_SRC
        | TextureUsages::COPY_DST;
    let time = |mut frame: Box<dyn FnMut() + '_>| {
        for _ in 0..10 {
            frame();
        }
        let mut v: Vec<Duration> = (0..120)
            .map(|_| {
                let t = Instant::now();
                frame();
                t.elapsed()
            })
            .collect();
        v.sort();
        (v[60].as_secs_f64() * 1e3, v[108].as_secs_f64() * 1e3)
    };
    for win in [(1920u32, 1080u32), (3840, 2160)] {
        for (label, s) in [("640x360", (640u32, 360u32)), ("full window", win)] {
            let (x, y) = ((win.0 - s.0) / 2, (win.1 - s.1) / 2);
            let surface = texture(&device, s, rt);
            let ui = texture(&device, win, rt);
            let above = texture(&device, s, rt);
            let target = texture(&device, win, rt);
            let present = texture(&device, win, TextureUsages::COPY_DST);
            let (sv, uv, av, tv) = (
                surface.create_view(&Default::default()),
                ui.create_view(&Default::default()),
                above.create_view(&Default::default()),
                target.create_view(&Default::default()),
            );
            let finish = |mut e: wgpu::CommandEncoder| {
                e.copy_texture_to_texture(
                    target.as_image_copy(),
                    present.as_image_copy(),
                    target.size(),
                );
                queue.submit(Some(e.finish()));
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            };
            let old = time(Box::new(|| {
                let mut e = device.create_command_encoder(&Default::default());
                layer.blit_layer_view(
                    &device,
                    &queue,
                    &mut e,
                    &sv,
                    &tv,
                    (x as f32, y as f32, s.0 as f32, s.1 as f32),
                    win,
                    Some(wgpu::Color::WHITE),
                    [0.0; 4],
                    None,
                );
                layer.overlay_ui(&device, &mut e, &ui, &tv);
                finish(e);
            }));
            fn draw<'a>(
                sv: &'a wgpu::TextureView,
                s: (u32, u32),
                (x, y): (u32, u32),
                above: Option<&'a wgpu::TextureView>,
            ) -> SurfaceDraw<'a> {
                SurfaceDraw {
                    texture: sv,
                    extents: s,
                    straight_alpha: false,
                    device_from_image: device_from_image(Affine::translate((x as f64, y as f64))),
                    own: Coverage::Rect([x, y, x + s.0, y + s.1]),
                    ancestors: Coverage::Rect([x, y, x + s.0, y + s.1]),
                    region: (x, y, s.0, s.1),
                    above_in: above,
                    above_out: None,
                    base: None,
                }
            }
            let empty = time(Box::new(|| {
                let mut e = device.create_command_encoder(&Default::default());
                split.composite(
                    &device,
                    &mut e,
                    &uv,
                    &tv,
                    win,
                    &[draw(&sv, s, (x, y), None)],
                );
                finish(e);
            }));
            let with_above = time(Box::new(|| {
                let mut e = device.create_command_encoder(&Default::default());
                split.composite(
                    &device,
                    &mut e,
                    &uv,
                    &tv,
                    win,
                    &[draw(&sv, s, (x, y), Some(&av))],
                );
                finish(e);
            }));
            // A video on the split path (#1865 PR 4): its new CPU frame
            // uploaded (`SplitFrame::upload_split_frames`), then composited.
            let pixels = vec![200u8; (s.0 * s.1 * 4) as usize];
            let video = time(Box::new(|| {
                queue.write_texture(
                    surface.as_image_copy(),
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(s.0 * 4),
                        rows_per_image: Some(s.1),
                    },
                    wgpu::Extent3d {
                        width: s.0,
                        height: s.1,
                        depth_or_array_layers: 1,
                    },
                );
                let mut e = device.create_command_encoder(&Default::default());
                split.composite(
                    &device,
                    &mut e,
                    &uv,
                    &tv,
                    win,
                    &[draw(&sv, s, (x, y), None)],
                );
                finish(e);
            }));
            let ms = |(m, p): (f64, f64)| format!("{m:5.2} / {p:5.2} ms");
            eprintln!("{}x{} {label}:", win.0, win.1);
            eprintln!("  old layer path (blit + overlay + copy)  {}", ms(old));
            eprintln!("  split, above empty                      {}", ms(empty));
            eprintln!(
                "  split, above cached                     {}",
                ms(with_above)
            );
            eprintln!("  split video (upload + composite)        {}", ms(video));
        }
    }
}

// ── The #1887 re-review's cases ────────────────────────────────────────────

/// A card's own background, painted before its clip opens.
fn card_background(s: &mut Scene, t: Affine) {
    s.fill(
        Fill::NonZero,
        t,
        SLATE,
        None,
        &RoundedRect::new(14.3, 6.6, 52.7, 44.2, 10.0),
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_rx_rounded_cards_and_transparent_windows() {
    for case in [
        Case {
            name: "RX1 transparent window, card, translucent frame, controls",
            ancestors: card(),
            before: nothing,
            texel: texel_straight,
            straight: true,
            after_in: controls,
            above_in: true,
            transparent: true,
            ..plain()
        },
        Case {
            name: "RX2 transparent window, card, slate in its clip",
            ancestors: card(),
            before: slate,
            transparent: true,
            ..plain()
        },
        Case {
            name: "RX3 transparent window, card, a wash in its clip",
            ancestors: card(),
            before: wash,
            transparent: true,
            ..plain()
        },
        Case {
            // The common card: its background behind its clip, nothing in
            // it beneath the surface. D8's halo is zero.
            name: "RX4 card with its own background, controls",
            ancestors: card(),
            before: nothing,
            under: card_background,
            after_in: controls,
            above_in: true,
            ..plain()
        },
        Case {
            name: "RX7 transparent window, card, translucent frame, a wash in its clip",
            ancestors: card(),
            before: wash,
            texel: texel_straight,
            straight: true,
            transparent: true,
            ..plain()
        },
    ] {
        assert_d10(&case);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_rx_content_between_nested_clips() {
    for case in [
        Case {
            name: "RX5 rounded card, slate between, a whole-pixel scroller inside",
            ancestors: vec![card()[0], Clip::Rect(Rect::new(8.0, 4.0, 70.0, 60.0))],
            before: nothing,
            between: slate,
            ..plain()
        },
        Case {
            name: "RX6 two rounded clips, slate between, the page inside",
            ancestors: vec![
                card()[0],
                Clip::Rounded(RoundedRect::new(14.8, 7.1, 52.2, 43.7, 9.5)),
            ],
            before: page,
            between: slate,
            ..plain()
        },
    ] {
        assert_d10(&case);
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_rx_region_past_the_window() {
    // R reaches past the window's right and bottom edges: the compositor
    // clamps it to the target.
    assert_d10(&Case {
        name: "RX8 R past the right and bottom",
        r: (60, 40, 40, 26),
        image: Affine::translate((60.0, 40.0)),
        // The sidebar after it is over it now.
        above_out: true,
        ..plain()
    });
}

/// N videos uploaded and composited in one frame (the #1906 review).
#[test]
#[ignore = "a measurement; needs a GPU adapter"]
fn split_multi_video_cost_probe() {
    use std::time::{Duration, Instant};
    let (device, queue, _errors) = device();
    let split = SplitCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let rt = TextureUsages::RENDER_ATTACHMENT
        | TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_SRC
        | TextureUsages::COPY_DST;
    for (win, s, n) in [
        ((1920u32, 1080u32), (960u32, 540u32), 1usize),
        ((1920, 1080), (960, 540), 2),
        ((1920, 1080), (960, 540), 4),
        ((3840, 2160), (1920, 1080), 1),
        ((3840, 2160), (1920, 1080), 4),
    ] {
        let ui = texture(&device, win, rt);
        let target = texture(&device, win, rt);
        let (uv, tv) = (
            ui.create_view(&Default::default()),
            target.create_view(&Default::default()),
        );
        let surfaces: Vec<_> = (0..n).map(|_| texture(&device, s, rt)).collect();
        let views: Vec<_> = surfaces
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let pos: Vec<(u32, u32)> = (0..n)
            .map(|i| ((i as u32 % 2) * s.0, (i as u32 / 2) * s.1))
            .collect();
        let pixels = vec![200u8; (s.0 * s.1 * 4) as usize];
        let frame = || {
            for t in &surfaces {
                queue.write_texture(
                    t.as_image_copy(),
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(s.0 * 4),
                        rows_per_image: Some(s.1),
                    },
                    wgpu::Extent3d {
                        width: s.0,
                        height: s.1,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let draws: Vec<SurfaceDraw<'_>> = views
                .iter()
                .zip(&pos)
                .map(|(v, &(x, y))| SurfaceDraw {
                    texture: v,
                    extents: s,
                    straight_alpha: true,
                    device_from_image: device_from_image(Affine::translate((x as f64, y as f64))),
                    own: Coverage::Rect([x, y, x + s.0, y + s.1]),
                    ancestors: Coverage::Rect([x, y, x + s.0, y + s.1]),
                    region: (x, y, s.0, s.1),
                    above_in: None,
                    above_out: None,
                    base: None,
                })
                .collect();
            let mut e = device.create_command_encoder(&Default::default());
            split.composite(&device, &mut e, &uv, &tv, win, &draws);
            queue.submit(Some(e.finish()));
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        };
        for _ in 0..10 {
            frame();
        }
        let mut v: Vec<Duration> = (0..120)
            .map(|_| {
                let t = Instant::now();
                frame();
                t.elapsed()
            })
            .collect();
        v.sort();
        eprintln!(
            "[rvcost] window {}x{}, {n} videos of {}x{}: {:.2} / {:.2} ms",
            win.0,
            win.1,
            s.0,
            s.1,
            v[60].as_secs_f64() * 1e3,
            v[108].as_secs_f64() * 1e3
        );
    }
}
