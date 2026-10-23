//! #1865 PR 1: the split compositor against the in-scene render, on slices
//! built by hand the way the split painter (PR 2) will build them. They need
//! an adapter, so they are `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_split_`.
//!
//! **The rule (D10).** Every pixel is bit-equal to in-scene except, each only
//! where the case's own renders say so ([`allowance`]):
//! - inside R, where the above slice is translucent or antialiased
//!   (0 < alpha < 255): one level;
//! - inside R, where the surface itself is (its texels' alpha times its
//!   clip-chain coverage, from the surface rendered alone): one level;
//! - inside R, every pixel the surface covers when it is resampled (not
//!   1:1 at whole pixels): one level, the float rounding order of Vello's
//!   fine shader against the compositor's;
//! - outside R, within one Vello tile (16 px) of it, where the content
//!   painted after the surface, or an ancestor clip's edge, is fractional:
//!   one level (Vello's 8-bit blend stack in tiles the inverse clip crosses);
//! - inside R, on an ancestor clip's antialiased edge: D8's bound,
//!   c(1 − c) × |what the clip holds − what is behind it|, plus one level.
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
/// classes, and exactly the cases that need the amendment fail (straight and
/// premultiplied translucent frames, scaled frames, a rounded ancestor clip):
/// the switch that would send those surfaces in-scene instead.
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
/// the window, a translation by −R's origin for the above slice).
type Paint = fn(&mut Scene, Affine);

fn nothing(_: &mut Scene, _: Affine) {}

/// A page: a translucent wash and a circle, some of it under the surface.
fn page(s: &mut Scene, t: Affine) {
    let window = Rect::new(0.0, 0.0, W as f64, H as f64);
    s.fill(Fill::NonZero, t, SLATE, None, &window);
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
    /// After the surface, outside every clip.
    after_out: Paint,
    /// `Some`: the image rect and every clip are whole-pixel rectangles and
    /// this is their intersection. `None`: a mask.
    cover: Option<[u32; 4]>,
    /// Whether the above slice must have something (the case is vacuous
    /// otherwise) or must be empty.
    above: bool,
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
    /// The above slice (R-sized), if it has anything.
    above: Option<Vec<[u8; 4]>>,
    /// Window-sized renders over transparent of: the after-content alone; the
    /// surface alone under its clips; the ancestors' clips filled white.
    after_alone: Vec<[u8; 4]>,
    surface_alone: Vec<[u8; 4]>,
    /// The image rect filled white under the surface's clips: its coverage.
    coverage_alone: Vec<[u8; 4]>,
    ancestors_alone: Vec<[u8; 4]>,
    /// The before-content, unclipped, over the white base: what an ancestor
    /// clip holds, against the white behind it (D8).
    before_unclipped: Vec<[u8; 4]>,
}

/// How far a split may move window pixel `(x, y)` from in-scene (D10), from
/// the case's own renders. `0` everywhere no class names.
fn allowance(case: &Case, split: &Split, x: u32, y: u32) -> i32 {
    let (rx, ry, rw, rh) = case.r;
    let r = Rect::new(rx as f64, ry as f64, (rx + rw) as f64, (ry + rh) as f64);
    let p = (x as f64 + 0.5, y as f64 + 0.5);
    let fractional = |a: u8| a != 0 && a != 255;
    let i = (y * W + x) as usize;
    if r.contains(p) {
        let local = ((y - ry) * rw + (x - rx)) as usize;
        let surface = split.surface_alone[i][3];
        let resampled = case.image.as_coeffs()[..4] != [1.0, 0.0, 0.0, 1.0]
            || case.image.as_coeffs()[4..].iter().any(|v| v.fract() != 0.0);
        let coverage = split.coverage_alone[i][3];
        let mut allow = 0;
        // Decided (D10): the above slice's and the mask's fractional pixels.
        if split
            .above
            .as_ref()
            .is_some_and(|a| fractional(a[local][3]))
            || fractional(coverage)
        {
            allow = 1;
        }
        // The amendment: translucent surface pixels, and resampled surfaces.
        if D10_AMENDMENT && (fractional(surface) || (resampled && surface != 0)) {
            allow = 1;
        }
        let c = split.ancestors_alone[i][3];
        if fractional(c) {
            let c = c as f64 / 255.0;
            let delta = split.before_unclipped[i][..3]
                .iter()
                .map(|v| 255 - *v as i32)
                .max()
                .unwrap() as f64;
            allow = allow.max((c * (1.0 - c) * delta).ceil() as i32 + 1);
        }
        allow
    } else if r.inflate(TILE, TILE).contains(p)
        && (fractional(split.after_alone[i][3])
            || (D10_AMENDMENT && fractional(split.ancestors_alone[i][3])))
    {
        1
    } else {
        0
    }
}

/// How a split is put together; the controls break one piece each.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// The above slice is not composited.
    NoAbove,
    /// The coverage rect is one pixel too narrow.
    NarrowCover,
    /// The texture's alpha type is passed the wrong way round.
    WrongAlpha,
}

fn run(case: &Case, fault: Fault) -> (Vec<[u8; 4]>, Split) {
    let (device, queue, errors) = device();
    let mut vello = renderer(&device);
    let (rx, ry, rw, rh) = case.r;
    let window = Rect::new(0.0, 0.0, W as f64, H as f64);
    let r = Rect::new(rx as f64, ry as f64, (rx + rw) as f64, (ry + rh) as f64);
    let to_r = Affine::translate((-(rx as f64), -(ry as f64)));
    let id = Affine::IDENTITY;

    // The surface's texture.
    let (ew, eh) = case.extents;
    let surface = texture(
        &device,
        case.extents,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
    );
    let texels: Vec<u8> = (0..eh)
        .flat_map(|y| (0..ew).flat_map(move |x| (case.texel)(x, y)))
        .collect();
    queue.write_texture(
        surface.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ew * 4),
            rows_per_image: Some(eh),
        },
        wgpu::Extent3d {
            width: ew,
            height: eh,
            depth_or_array_layers: 1,
        },
    );

    // In-scene: what #1861 draws — the image through rinch's own painter.
    let mut image = vello.register_texture(surface.clone());
    if !case.straight {
        image.alpha_type = ImageAlphaType::AlphaPremultiplied;
    }
    let mut painter = rinch_dom::paint::vello_painter::VelloPainter::new();
    rinch_dom::paint::painter::Painter::draw_gpu_image(&mut painter, &image, case.image);
    let mut in_scene = Scene::new();
    for c in &case.ancestors {
        c.push(&mut in_scene, id);
    }
    (case.before)(&mut in_scene, id);
    if let Some(own) = case.own {
        in_scene.push_clip_layer(Fill::NonZero, id, &own);
    }
    in_scene.append(painter.scene(), None);
    if case.own.is_some() {
        in_scene.pop_layer();
    }
    (case.after_in)(&mut in_scene, id);
    for _ in &case.ancestors {
        in_scene.pop_layer();
    }
    (case.after_out)(&mut in_scene, id);
    let want = read(
        &device,
        &queue,
        &render(&device, &queue, &mut vello, &in_scene, (W, H), Color::WHITE),
    );

    // Below: after-content kept out of R by the inverse clip, popped and
    // pushed again around each ancestor's pop.
    let not_r = inverse(window, r);
    let mut below = Scene::new();
    for c in &case.ancestors {
        c.push(&mut below, id);
    }
    (case.before)(&mut below, id);
    below.push_clip_layer(Fill::NonZero, id, &not_r);
    (case.after_in)(&mut below, id);
    for _ in &case.ancestors {
        below.pop_layer();
        below.pop_layer();
        below.push_clip_layer(Fill::NonZero, id, &not_r);
    }
    (case.after_out)(&mut below, id);
    below.pop_layer();
    let below = render(&device, &queue, &mut vello, &below, (W, H), Color::WHITE);

    // Above: the after-content inside R, in R's coordinates; the ancestors
    // replayed, R pushed as a clip.
    let rect_r = Rect::new(0.0, 0.0, rw as f64, rh as f64);
    let mut above = Scene::new();
    for c in &case.ancestors {
        c.push(&mut above, to_r);
    }
    above.push_clip_layer(Fill::NonZero, id, &rect_r);
    (case.after_in)(&mut above, to_r);
    for _ in &case.ancestors {
        above.pop_layer();
        above.pop_layer();
        above.push_clip_layer(Fill::NonZero, id, &rect_r);
    }
    (case.after_out)(&mut above, to_r);
    above.pop_layer();
    let above_tex = render(
        &device,
        &queue,
        &mut vello,
        &above,
        (rw, rh),
        Color::TRANSPARENT,
    );
    let above_px = read(&device, &queue, &above_tex);
    let above_used = above_px.iter().any(|p| p[3] != 0);

    // The mask: the image rect's coverage under the clip chain.
    let mask_tex = case.cover.is_none().then(|| {
        let mut mask = Scene::new();
        for c in &case.ancestors {
            c.push(&mut mask, to_r);
        }
        if let Some(own) = case.own {
            mask.push_clip_layer(Fill::NonZero, to_r, &own);
        }
        mask.fill(
            Fill::NonZero,
            to_r * case.image,
            Color::WHITE,
            None,
            &Rect::new(0.0, 0.0, ew as f64, eh as f64),
        );
        if case.own.is_some() {
            mask.pop_layer();
        }
        for _ in &case.ancestors {
            mask.pop_layer();
        }
        render(
            &device,
            &queue,
            &mut vello,
            &mask,
            (rw, rh),
            Color::TRANSPARENT,
        )
    });

    // The class renders: the surface alone under its clips, the ancestors'
    // coverage, the before-content unclipped over white.
    let mut alone = Scene::new();
    for c in &case.ancestors {
        c.push(&mut alone, id);
    }
    if let Some(own) = case.own {
        alone.push_clip_layer(Fill::NonZero, id, &own);
    }
    alone.append(painter.scene(), None);
    if case.own.is_some() {
        alone.pop_layer();
    }
    for _ in &case.ancestors {
        alone.pop_layer();
    }
    let surface_alone = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &alone,
            (W, H),
            Color::TRANSPARENT,
        ),
    );
    let mut cov = Scene::new();
    for c in &case.ancestors {
        c.push(&mut cov, id);
    }
    if let Some(own) = case.own {
        cov.push_clip_layer(Fill::NonZero, id, &own);
    }
    cov.fill(
        Fill::NonZero,
        case.image,
        Color::WHITE,
        None,
        &Rect::new(0.0, 0.0, ew as f64, eh as f64),
    );
    if case.own.is_some() {
        cov.pop_layer();
    }
    for _ in &case.ancestors {
        cov.pop_layer();
    }
    let coverage_alone = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &cov,
            (W, H),
            Color::TRANSPARENT,
        ),
    );
    let mut anc = Scene::new();
    for c in &case.ancestors {
        c.push(&mut anc, id);
    }
    anc.fill(Fill::NonZero, id, Color::WHITE, None, &window);
    for _ in &case.ancestors {
        anc.pop_layer();
    }
    let ancestors_alone = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &anc,
            (W, H),
            Color::TRANSPARENT,
        ),
    );
    let mut unclipped = Scene::new();
    (case.before)(&mut unclipped, id);
    let before_unclipped = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &unclipped,
            (W, H),
            Color::WHITE,
        ),
    );

    // What the after-content alone covers, for the ring class.
    let mut after = Scene::new();
    for c in &case.ancestors {
        c.push(&mut after, id);
    }
    (case.after_in)(&mut after, id);
    for _ in &case.ancestors {
        after.pop_layer();
    }
    (case.after_out)(&mut after, id);
    let after_alone = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &after,
            (W, H),
            Color::TRANSPARENT,
        ),
    );

    let target = texture(
        &device,
        (W, H),
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let compositor = SplitCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let surface_view = surface.create_view(&Default::default());
    let above_view = above_tex.create_view(&Default::default());
    let mask_view = mask_tex
        .as_ref()
        .map(|m| m.create_view(&Default::default()));
    let coverage = match (&mask_view, case.cover) {
        (Some(m), _) => Coverage::Mask(m),
        (None, Some(c)) => Coverage::Rect(if fault == Fault::NarrowCover {
            [c[0], c[1], c[2] - 1, c[3]]
        } else {
            c
        }),
        (None, None) => unreachable!(),
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    compositor.composite(
        &device,
        &mut encoder,
        &below.create_view(&Default::default()),
        &target.create_view(&Default::default()),
        (W, H),
        &[SurfaceDraw {
            texture: &surface_view,
            extents: case.extents,
            straight_alpha: case.straight != (fault == Fault::WrongAlpha),
            device_from_image: device_from_image(case.image),
            coverage,
            region: case.r,
            above: (above_used && fault != Fault::NoAbove).then_some(&above_view),
        }],
    );
    queue.submit(Some(encoder.finish()));
    let composited = read(&device, &queue, &target);
    vello.unregister_texture(image);
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
            above: above_used.then_some(above_px),
            after_alone,
            surface_alone,
            coverage_alone,
            ancestors_alone,
            before_unclipped,
        },
    )
}

/// The pixels breaking D10: `(x, y, split, in-scene, allowance)`.
fn violations(case: &Case, fault: Fault) -> Vec<Violation> {
    let (want, split) = run(case, fault);
    let mut out = Vec::new();
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let (got, want) = (split.composited[i], want[i]);
            if got == want {
                continue;
            }
            let off = (0..4)
                .map(|c| (got[c] as i32 - want[c] as i32).abs())
                .max()
                .unwrap();
            let allow = allowance(case, &split, x, y);
            if off > allow {
                out.push((x, y, got, want, allow));
            }
        }
    }
    out
}

fn assert_d10(case: &Case) {
    let (_, split) = run(case, Fault::None);
    assert_eq!(
        split.above.is_some(),
        case.above,
        "{}: the above slice is not as the case says",
        case.name
    );
    let wrong = violations(case, Fault::None);
    let allowed = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| allowance(case, &split, x, y) > 0)
        .count();
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
        cover: Some([20, 12, 60, 38]),
        above: false,
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_an_opaque_frame_is_exact() {
    assert_d10(&plain());
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_straight_alpha_frame_over_the_page_is_exact() {
    assert_d10(&Case {
        name: "straight-alpha frame over a solid and a translucent page",
        texel: texel_straight,
        straight: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_premultiplied_translucent_frame_is_exact() {
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
        name: "scrolled half out of a whole-pixel scroller",
        ancestors: vec![Clip::Rect(Rect::new(8.0, 20.0, 70.0, 60.0))],
        // The region is the content box; the clip is the cover's business.
        cover: Some([20, 20, 60, 38]),
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_its_own_radius_is_a_mask() {
    assert_d10(&Case {
        name: "own inner radius",
        own: Some(RoundedRect::new(20.0, 12.0, 60.0, 38.0, 7.0)),
        cover: None,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_rounded_ancestor_clip_is_a_mask() {
    // The clipping ancestor paints no background, where D8's halo is zero:
    // its ring is then held to the mask's one level like any edge.
    assert_d10(&Case {
        name: "rounded, fractional ancestor clip across the frame",
        ancestors: vec![Clip::Rounded(RoundedRect::new(14.3, 6.6, 52.7, 44.2, 10.0))],
        cover: None,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_over_the_frame_are_the_above_slice() {
    assert_d10(&Case {
        name: "a control bar straddling R",
        after_in: controls,
        above: true,
        ..plain()
    });
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_inside_a_rounded_card() {
    assert_d10(&Case {
        name: "controls inside a rounded card, own radius too",
        ancestors: vec![Clip::Rounded(RoundedRect::new(12.0, 8.0, 80.0, 56.0, 6.0))],
        own: Some(RoundedRect::new(20.0, 12.0, 60.0, 38.0, 5.0)),
        after_in: controls,
        above: true,
        cover: None,
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

// ── Positive controls: each fault must be seen, outside the tolerance ──────

fn assert_fault_seen(case: &Case, fault: Fault, what: &str) {
    assert!(
        !violations(case, fault).is_empty(),
        "control ({what}): no pixel past its allowance; the oracle cannot see it"
    );
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_controls_the_oracle_sees_each_fault() {
    let controls = Case {
        name: "a control bar straddling R",
        after_in: controls,
        above: true,
        ..plain()
    };
    assert_fault_seen(&controls, Fault::NoAbove, "above slice skipped");
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
                    coverage: Coverage::Rect([x, y, x + s.0, y + s.1]),
                    region: (x, y, s.0, s.1),
                    above,
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
            let ms = |(m, p): (f64, f64)| format!("{m:5.2} / {p:5.2} ms");
            eprintln!("{}x{} {label}:", win.0, win.1);
            eprintln!("  old layer path (blit + overlay + copy)  {}", ms(old));
            eprintln!("  split, above empty                      {}", ms(empty));
            eprintln!(
                "  split, above cached                     {}",
                ms(with_above)
            );
        }
    }
}
