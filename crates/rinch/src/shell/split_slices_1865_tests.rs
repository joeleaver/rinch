//! #1865: the two Vello facts the split-layer design stands on, pinned before
//! anything is built on them (`docs/design/split-layer-gpu-surfaces.md`,
//! *Risks* 1 and 2). They need an adapter, so they are `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_split_`.
//!
//! 1. A texture registered with the renderer is read only by a scene that
//!    draws it, so every GPU surface can stay registered (for an on-the-spot
//!    in-scene fallback) without paying the atlas copy when it is split.
//! 2. A clip whose edges lie on whole device pixels has coverage exactly 0 or
//!    1, so the below slice can exclude the surface's region R with an inverse
//!    clip — cut into segments around the ancestors' pops — and change no
//!    pixel outside R.
use std::sync::{Arc, Mutex};

use vello::kurbo::{Affine, BezPath, Rect, RoundedRect, Shape};
use vello::peniko::{Color, Fill, ImageBrush};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use wgpu::{Device, Queue, Texture, TextureFormat, TextureUsages};

const W: u32 = 64;
const H: u32 = 48;

/// The device, and the validation errors it raised since it was made.
pub(super) fn device() -> (Device, Queue, Arc<Mutex<Vec<String>>>) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("a GPU adapter (these fixtures are #[ignore]d for hosts without one)");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device");
    let errors = Arc::new(Mutex::new(Vec::new()));
    let sink = errors.clone();
    device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        sink.lock().unwrap().push(e.to_string())
    }));
    (device, queue, errors)
}

pub(super) fn renderer(device: &Device) -> Renderer {
    Renderer::new(
        device,
        RendererOptions {
            antialiasing_support: AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("vello")
}

fn target(device: &Device, usage: TextureUsages) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split_slices_1865_tests"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
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

fn render(
    device: &Device,
    queue: &Queue,
    renderer: &mut Renderer,
    scene: &Scene,
    aa: AaConfig,
) -> Vec<[u8; 4]> {
    let out = target(
        device,
        TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
    );
    let view = out.create_view(&Default::default());
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &view,
            &RenderParams {
                base_color: Color::WHITE,
                width: W,
                height: H,
                antialiasing_method: aa,
            },
        )
        .expect("render");
    read(device, queue, &out)
}

fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
    let row = W * 4; // 256: already the copy alignment
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
    data.as_chunks::<4>().0.to_vec()
}

const AA: [(AaConfig, &str); 2] = [(AaConfig::Msaa16, "msaa16"), (AaConfig::Area, "area")];

// ── 1. A registered texture no scene draws is never read ──────────────────

/// A destroyed texture still registered: a render whose scene does not draw
/// it raises no error, so Vello did not copy it into the atlas; the control,
/// a scene that does draw it, raises one — the probe can see a read.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_registered_texture_no_scene_draws_is_not_read() {
    let (device, queue, errors) = device();
    let mut renderer = renderer(&device);
    let texture = target(
        &device,
        TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    );
    let image = renderer.register_texture(texture.clone());
    texture.destroy();

    let mut without = Scene::new();
    without.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(200, 30, 30),
        None,
        &Rect::new(4.0, 4.0, 20.0, 20.0),
    );
    for _ in 0..3 {
        render(&device, &queue, &mut renderer, &without, AaConfig::Msaa16);
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert_eq!(
        *errors.lock().unwrap(),
        Vec::<String>::new(),
        "a scene that does not draw the registered texture read it"
    );

    // Control: the same renderer, a scene that draws the image.
    let mut with = Scene::new();
    with.draw_image(&ImageBrush::new(image), Affine::IDENTITY);
    let out = target(
        &device,
        TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
    );
    let view = out.create_view(&Default::default());
    let _ = renderer.render_to_texture(
        &device,
        &queue,
        &with,
        &view,
        &RenderParams {
            base_color: Color::WHITE,
            width: W,
            height: H,
            antialiasing_method: AaConfig::Msaa16,
        },
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert!(
        !errors.lock().unwrap().is_empty(),
        "control: a scene that draws the destroyed texture raised no error, \
         so the probe cannot see a read"
    );
}

// ── 2. Whole-pixel clips have binary coverage ─────────────────────────────

/// `outer` with `hole` cut out: the inverse clip of the below slice, filled
/// **non-zero**, the hole wound the other way. Not even-odd: on lavapipe an
/// even-odd clip nested in another clip drops coverage it has (pixels left
/// of R lost their after-content), where the reversed winding is right on
/// both lavapipe and RADV.
pub(super) fn inverse(outer: Rect, hole: Rect) -> BezPath {
    let mut path = outer.to_path(0.1);
    path.move_to((hole.x0, hole.y0));
    path.line_to((hole.x0, hole.y1));
    path.line_to((hole.x1, hole.y1));
    path.line_to((hole.x1, hole.y0));
    path.close_path();
    path
}

const RED: Color = Color::from_rgb8(220, 20, 60);
const TEAL: Color = Color::from_rgba8(0, 128, 128, 160);
const GOLD: Color = Color::from_rgb8(250, 200, 0);
const NAVY: Color = Color::from_rgba8(0, 0, 128, 200);

/// A clip on whole device pixels — direct, inverse, and under an
/// integer translation — leaves every pixel either fully inside or fully
/// outside: each is exactly the fill or exactly the base.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_a_whole_pixel_clip_has_binary_coverage() {
    let (device, queue, errors) = device();
    let mut renderer = renderer(&device);
    let r = Rect::new(13.0, 7.0, 41.0, 30.0);
    let window = Rect::new(0.0, 0.0, W as f64, H as f64);
    let fill = Color::from_rgb8(220, 20, 60);
    let cases: [(&str, Affine, BezPath, Fill); 3] = [
        ("rect", Affine::IDENTITY, r.to_path(0.1), Fill::NonZero),
        (
            "inverse",
            Affine::IDENTITY,
            inverse(window, r),
            Fill::NonZero,
        ),
        (
            "translated",
            Affine::translate((5.0, -3.0)),
            r.to_path(0.1),
            Fill::NonZero,
        ),
    ];
    for (aa, aa_name) in AA {
        for (name, transform, clip, rule) in &cases {
            let mut scene = Scene::new();
            scene.push_clip_layer(*rule, *transform, clip);
            scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &window);
            scene.pop_layer();
            let px = render(&device, &queue, &mut renderer, &scene, aa);
            let partial: Vec<(u32, u32, [u8; 4])> = px
                .iter()
                .enumerate()
                .filter(|(_, p)| **p != [255, 255, 255, 255] && **p != [220, 20, 60, 255])
                .map(|(i, p)| (i as u32 % W, i as u32 / W, *p))
                .collect();
            assert!(
                partial.is_empty(),
                "{aa_name} {name}: pixels with partial coverage: {:?}",
                &partial[..partial.len().min(8)]
            );
            // Not vacuous: both sides of the edge are present.
            assert!(px.contains(&[255, 255, 255, 255]) && px.contains(&[220, 20, 60, 255]));
        }
        // Control, off the fixed point: half a pixel off the grid, the same
        // clip leaves partial pixels — the check above can fail.
        let mut scene = Scene::new();
        let off = Rect::new(13.5, 7.0, 41.0, 30.0);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &off);
        scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &window);
        scene.pop_layer();
        let px = render(&device, &queue, &mut renderer, &scene, aa);
        assert!(
            px.iter()
                .any(|p| *p != [255, 255, 255, 255] && *p != [220, 20, 60, 255]),
            "{aa_name} control: a clip at x = 13.5 left no partial pixel"
        );
    }
    assert_eq!(*errors.lock().unwrap(), Vec::<String>::new());
}

/// A pixel where below is not what it must be: `(x, y, below, want)`.
type Mismatch = (u32, u32, [u8; 4], [u8; 4]);

/// The ancestor clip `A` of [`below_against_in_scene`]: rounded and off the
/// pixel grid, so its own edge pixels have fractional coverage — the case a
/// split must not disturb.
fn ancestor_clip() -> RoundedRect {
    RoundedRect::new(3.3, 2.6, 58.7, 44.2, 9.0)
}

/// The below slice's construction against the in-scene render it stands in
/// for, with Msaa16 (the desktop GPU window's). The ancestor clip `A` is open
/// when the surface is reached; `before` was painted inside it, `inside_a`
/// after the surface inside it, and `later` after it outside every clip.
/// Below cuts the after-content out of `r` with an inverse clip, popped and
/// re-pushed around `A`'s pop.
///
/// Returns the pixels where below differs from what it must be — the
/// in-scene render outside `r`, the before-content alone inside it (what the
/// surface is drawn over) — as `(x, y, below, want)`, and how many pixels lie
/// on `A`'s fractional edge (so a pass is not vacuous).
fn below_against_in_scene(
    device: &Device,
    queue: &Queue,
    renderer: &mut Renderer,
    r: Rect,
) -> (Vec<Mismatch>, usize) {
    let window = Rect::new(0.0, 0.0, W as f64, H as f64);
    let before = |s: &mut Scene| s.fill(Fill::NonZero, Affine::IDENTITY, TEAL, None, &window);
    // A bar straddling R's bottom edge and A's rounded corner, its top edge
    // on a half pixel (a row of fractional coverage).
    let inside_a = |s: &mut Scene| {
        s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            RED,
            None,
            &RoundedRect::new(0.0, 24.5, 50.0, 46.0, 4.0),
        );
    };
    let later = |s: &mut Scene| {
        s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            NAVY,
            None,
            &Rect::new(30.25, 1.5, 62.0, 20.75),
        );
        s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            GOLD,
            None,
            &vello::kurbo::Circle::new((20.0, 14.0), 6.3),
        );
    };
    let not_r = inverse(window, r);
    let a = ancestor_clip();

    let mut in_scene = Scene::new();
    in_scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &a);
    before(&mut in_scene);
    inside_a(&mut in_scene);
    in_scene.pop_layer();
    later(&mut in_scene);

    let mut below = Scene::new();
    below.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &a);
    before(&mut below);
    // The split: inverse-R pushed on top of the open stack.
    below.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &not_r);
    inside_a(&mut below);
    // A's pop: inverse-R off, A off, inverse-R back.
    below.pop_layer();
    below.pop_layer();
    below.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &not_r);
    later(&mut below);
    below.pop_layer();

    let mut before_only = Scene::new();
    before_only.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &a);
    before(&mut before_only);
    before_only.pop_layer();

    let want_out = render(device, queue, renderer, &in_scene, AaConfig::Msaa16);
    let got = render(device, queue, renderer, &below, AaConfig::Msaa16);
    let want_in = render(device, queue, renderer, &before_only, AaConfig::Msaa16);
    let interior = want_in[(24 * W + 32) as usize];
    let mut wrong = Vec::new();
    let mut fractional_a = 0;
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let inside = r.contains((x as f64 + 0.5, y as f64 + 0.5));
            let want = if inside { want_in[i] } else { want_out[i] };
            if got[i] != want {
                wrong.push((x, y, got[i], want));
            }
            if want_in[i] != [255, 255, 255, 255] && want_in[i] != interior {
                fractional_a += 1;
            }
        }
    }
    (wrong, fractional_a)
}

/// Below against in-scene, with R on whole pixels (on and off Vello's 16-px
/// tile grid). Bit-exact everywhere except **one 8-bit level** on pixels near
/// R's edge (within a tile of it, outside R): where the inverse clip's edge
/// runs through a tile Vello pushes a real clip layer there, and its 8-bit
/// blend stack rounds translucent and antialiased after-content once more
/// than in-scene. Coverage itself is binary
/// (`gpu_split_a_whole_pixel_clip_has_binary_coverage`); the error is that
/// rounding. A tile-aligned R does not avoid it: an edge on a tile boundary
/// still puts segments in the neighbouring tile.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_the_below_slice_is_in_scene_but_one_level_near_r() {
    let (device, queue, errors) = device();
    let mut renderer = renderer(&device);
    let mut rounded = 0;
    for r in [
        Rect::new(10.0, 5.0, 44.0, 31.0),
        Rect::new(1.0, 30.0, 40.0, 47.0),
        Rect::new(0.0, 0.0, 48.0, 32.0),
        Rect::new(16.0, 16.0, 48.0, 48.0),
        // Across A's rounded bottom-left corner.
        Rect::new(0.0, 32.0, 48.0, 48.0),
        Rect::new(16.0, 0.0, 32.0, 48.0),
    ] {
        let (wrong, fractional_a) = below_against_in_scene(&device, &queue, &mut renderer, r);
        assert!(fractional_a > 0, "no fractional pixel on A's edge");
        let near = r.inflate(16.0, 16.0);
        for (x, y, got, want) in &wrong {
            let off = (0..4)
                .map(|c| (got[c] as i32 - want[c] as i32).abs())
                .max()
                .unwrap();
            let p = (*x as f64 + 0.5, *y as f64 + 0.5);
            assert!(
                off <= 1 && near.contains(p) && !r.contains(p),
                "R {r:?}: ({x}, {y}) {got:?} against {want:?}"
            );
        }
        rounded += wrong.len();
    }
    // Adapter-dependent: RADV (Radeon 890M) rounds a few such pixels, lavapipe
    // none. Reported, not asserted.
    eprintln!("pixels rounded by one level near R: {rounded}");
    assert_eq!(*errors.lock().unwrap(), Vec::<String>::new());
}

/// Control: R half a pixel off the grid makes errors far past one level —
/// the comparison above can fail.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_split_r_off_the_pixel_grid_breaks_the_below_slice() {
    let (device, queue, _errors) = device();
    let mut renderer = renderer(&device);
    let r = Rect::new(10.0, 5.5, 44.0, 31.0);
    let (wrong, _) = below_against_in_scene(&device, &queue, &mut renderer, r);
    let worst = wrong
        .iter()
        .map(|(_, _, g, w)| {
            (0..4)
                .map(|c| (g[c] as i32 - w[c] as i32).abs())
                .max()
                .unwrap()
        })
        .max()
        .unwrap_or(0);
    assert!(worst > 8, "R at y0 = 5.5: worst error {worst}");
}

/// Render `scene` into a new `(w, h)` target over transparent, read it back.
fn render_sized(
    device: &Device,
    queue: &Queue,
    renderer: &mut Renderer,
    scene: &Scene,
    (w, h): (u32, u32),
) -> Vec<[u8; 4]> {
    let out = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tile phase"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &out.create_view(&Default::default()),
            &RenderParams {
                base_color: Color::TRANSPARENT,
                width: w,
                height: h,
                antialiasing_method: AaConfig::Msaa16,
            },
        )
        .expect("render");
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        out.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        out.size(),
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

/// A measurement, not a test (#1894 review, item 4): does a slice's scene,
/// encoded where the window draws it, render the same moved toward the
/// origin by whole Vello tiles (16 px), so the window could render it into a
/// target covering R's tiles rather than one reaching R's far corner?
/// Measured 2026-10-25 on RADV and lavapipe alike: below 1024 px exact both
/// by whole tiles and by 31 px; at a 4K window's far side (3200, 1920) and
/// (3776, 2112) one and two pixels differ, up to 24 levels, by whole tiles
/// and by 31 px alike: f32 magnitude, not tile phase. So the crop is not
/// exact and the window renders slices in place.
/// `cargo test -p rinch --features gpu --lib split_slice_shift_probe -- --ignored --nocapture`
#[test]
#[ignore = "a measurement; needs a GPU adapter"]
fn split_slice_shift_probe() {
    let (device, queue, errors) = device();
    let mut renderer = renderer(&device);
    // A slice's content near `at`: curves at fractional coordinates, a
    // fractional rect, a translucent fill, a stroke.
    let content = |at: (f64, f64)| {
        let mut s = Scene::new();
        let t = Affine::translate(at);
        s.fill(
            Fill::NonZero,
            t,
            TEAL,
            None,
            &RoundedRect::new(3.3, 2.6, 58.7, 44.2, 9.7),
        );
        s.fill(
            Fill::NonZero,
            t,
            GOLD,
            None,
            &vello::kurbo::Circle::new((31.37, 22.81), 13.29),
        );
        s.fill(
            Fill::NonZero,
            t,
            RED,
            None,
            &Rect::new(10.25, 30.6, 50.75, 41.1),
        );
        s.stroke(
            &vello::kurbo::Stroke::new(1.3),
            t,
            NAVY,
            None,
            &vello::kurbo::Circle::new((20.6, 15.4), 7.71),
        );
        s
    };
    let (w, h) = (64u32, 48u32);
    for at in [
        (3200.0, 1920.0),
        (3776.0, 2112.0),
        (48.0, 16.0),
        (1024.0, 512.0),
    ] {
        // The window's render: the content where it is, a target reaching it.
        let far = (at.0 as u32 + w, at.1 as u32 + h);
        let full = render_sized(&device, &queue, &mut renderer, &content(at), far);
        let window: Vec<[u8; 4]> = (0..h)
            .flat_map(|y| {
                let full = &full;
                (0..w).map(move |x| full[((at.1 as u32 + y) * far.0 + at.0 as u32 + x) as usize])
            })
            .collect();
        for (shift, whole_tiles) in [(at, true), ((at.0 - 31.0, at.1 - 31.0), false)] {
            // The slice moved by `shift` as `Scene::append` moves it (the
            // window's encoding appended under a translation).
            let mut moved = Scene::new();
            moved.append(&content(at), Some(Affine::translate((-shift.0, -shift.1))));
            let (ox, oy) = ((at.0 - shift.0) as u32, (at.1 - shift.1) as u32);
            let small = render_sized(&device, &queue, &mut renderer, &moved, (ox + w, oy + h));
            let got: Vec<[u8; 4]> = (0..h)
                .flat_map(|y| {
                    let small = &small;
                    (0..w).map(move |x| small[((oy + y) * (ox + w) + ox + x) as usize])
                })
                .collect();
            let differ = got.iter().zip(&window).filter(|(a, b)| a != b).count();
            let worst = got
                .iter()
                .zip(&window)
                .map(|(a, b)| {
                    (0..4)
                        .map(|c| (a[c] as i32 - b[c] as i32).abs())
                        .max()
                        .unwrap()
                })
                .max()
                .unwrap();
            eprintln!(
                "PHASE at {at:?} shift {shift:?} whole_tiles={whole_tiles}: {differ} differ, worst {worst}"
            );
        }
    }
    assert_eq!(*errors.lock().unwrap(), Vec::<String>::new());
}
