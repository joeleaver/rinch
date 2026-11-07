//! #2286 stage 1 on a real adapter (CI's `test-gpu (lavapipe)`): the window
//! draws its scene with area coverage (1a), and light text is darkened on the
//! GPU painter as on the software one (1b).
//!
//! The text is the bundled Inter as `sans-serif` (`perf_expect::new_app`), at
//! a declared size and line height.

use super::*;
use crate::gpu_antialiasing::{GpuAntialiasing, TestAa};
use crate::shell::desktop::WgpuRenderer;
use rinch_dom::paint::text_rasterization::{
    ResolvedTextRasterization, StemDarkening, SystemTextSettings, TextRasterization,
    reset_thread_policy, set_thread_policy,
};
use std::sync::Arc;

const W: u32 = 320;
const H: u32 = 96;
const BPR: u32 = (W * 4).div_ceil(256) * 256;

/// Four lines of 13px/20px text in `fg` on `bg`, filling the window.
fn text_app(fg: &'static str, bg: &'static str, w: u32, h: u32) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            &format!(
                "width: {w}px; height: {h}px; padding: 6px; box-sizing: border-box; \
                 font: 13px/20px sans-serif; color: {fg}; background: {bg}"
            ),
        );
        let words = "Sphinx of black quartz, judge my vow. 0123456789 Hamburgefonstiv ";
        let t = scope.create_text(&words.repeat((w * h / 4000).max(4) as usize));
        root.append_child(&t);
        root
    });
    app.mount_component(w as f32, h as f32);
    app.resolve_and_repaint(w as f32, h as f32);
    app
}

fn gpu() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    (Arc::new(device), Arc::new(queue))
}

/// The window's UI texture after one full frame of `app`, rendered by the
/// shell's own renderer (`paint_frame`), as RGBA rows of `W` pixels.
fn render_window(app: &mut RinchApp) -> Vec<u8> {
    let (device, queue) = gpu();
    let mut r = WgpuRenderer::offscreen(
        device.clone(),
        queue.clone(),
        W,
        H,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let frame = app.build_gpu_frame(
        1.0,
        (W, H),
        Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
    );
    assert!(
        matches!(frame, GpuFrame::Full(_)),
        "the first frame is full"
    );
    r.paint_frame(frame, false).unwrap();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (BPR * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        r.render_texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(BPR),
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
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        out.extend_from_slice(&data[(y * BPR) as usize..(y * BPR + W * 4) as usize]);
    }
    out
}

/// The red channel's coverage of `fg` over `bg` (both pure black or white):
/// `0` is ground, `1` is ink.
fn coverage(rgba: &[u8], light_text: bool) -> Vec<f64> {
    rgba.chunks(4)
        .map(|p| {
            let v = p[0] as f64 / 255.0;
            if light_text { v } else { 1.0 - v }
        })
        .collect()
}

/// The window draws text with area coverage (#2286 1a): an edge takes any of
/// ~240 levels, where `Msaa16` gives at most 15 between ground and ink (it
/// counts 16 samples a pixel). Black text on white, the red channel's
/// distinct partial values over the whole window.
#[test]
#[ignore]
fn gpu_the_window_draws_text_with_area_coverage_2286() {
    // The production default; this crate's test build defaults to Msaa16.
    let _aa = TestAa::pin(GpuAntialiasing::Area);
    let mut app = text_app("#000000", "#ffffff", W, H);
    let rgba = render_window(&mut app);
    let partial: std::collections::BTreeSet<u8> = rgba
        .chunks(4)
        .map(|p| p[0])
        .filter(|&v| v != 0 && v != 255)
        .collect();
    let inked = rgba.chunks(4).filter(|p| p[0] < 128).count();
    assert!(inked > 500, "no text was drawn: {inked} dark pixels");
    assert!(
        partial.len() > 64,
        "edges took only {} distinct partial levels, an MSAA count (Msaa16 gives at \
         most 15): {partial:?}",
        partial.len()
    );
}

/// Light text is darkened on the GPU painter (#2286 1b): white on black
/// carries more ink than the same text black on white, and with darkening off
/// the two are each other's inverse, as both blend in encoded sRGB.
#[test]
#[ignore]
fn gpu_light_text_is_darkened_and_off_turns_it_off_2286() {
    let _aa = TestAa::pin(GpuAntialiasing::Area);
    let render = |light: bool, policy: ResolvedTextRasterization| {
        set_thread_policy(policy);
        let (fg, bg) = if light {
            ("#ffffff", "#000000")
        } else {
            ("#000000", "#ffffff")
        };
        let mut app = text_app(fg, bg, W, H);
        let rgba = render_window(&mut app);
        reset_thread_policy();
        coverage(&rgba, light)
    };
    // Two renders of one scene under area coverage are not bit-identical on a
    // parallel GPU: a tile's segments are ordered by `atomicAdd` and the area
    // fill sums their float coverage in that order (paint-and-damage.md,
    // "GPU antialiasing"). So a scene against itself is compared within
    // 1/255, and light against dark within 2/255 (that, plus the inversion's
    // own rounding).
    let worst = |a: &[f64], b: &[f64]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max)
    };
    let sum = |c: &[f64]| c.iter().sum::<f64>();
    let off = TextRasterization::default()
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default());
    let on = ResolvedTextRasterization::default();
    let dark = render(false, on);
    assert!(sum(&dark) > 500.0, "no text was drawn: ink {}", sum(&dark));
    let dark_off = render(false, off);
    let d = worst(&dark, &dark_off);
    assert!(
        d <= 1.0 / 255.0 + 1e-9,
        "dark text moved with darkening: by {d} somewhere"
    );
    let light_off = render(true, off);
    let d = worst(&light_off, &dark_off);
    assert!(
        d <= 2.0 / 255.0 + 1e-9,
        "with darkening off light text should be dark text's inverse; they differ by {d}"
    );
    let gain = sum(&render(true, on)) / sum(&dark);
    assert!(
        (1.08..1.35).contains(&gain),
        "light text should carry visibly more ink than dark text by default: {gain:.4}x"
    );
}

/// What a full page of text costs the GPU per AA mode and with darkening on
/// and off (`--ignored --nocapture`; CI's `test-gpu` skips it by name). The
/// frame is `render_to_texture` plus the wait, median of 20, after 3 warm-up
/// renders, on whatever adapter wgpu picks (lavapipe in CI's image).
#[test]
#[ignore]
fn text_aa_cost_probe_2286() {
    const PW: u32 = 1280;
    const PH: u32 = 800;
    let (device, queue) = gpu();
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: PW,
            height: PH,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for (scheme, fg, bg) in [
        ("dark-on-light", "#232629", "#ffffff"),
        ("light-on-dark", "#fcfcfc", "#202326"),
    ] {
        for darkening in [true, false] {
            let policy = if darkening {
                ResolvedTextRasterization::default()
            } else {
                ResolvedTextRasterization::default().with_darkening(0.0)
            };
            set_thread_policy(policy);
            let mut app = text_app(fg, bg, PW, PH);
            let scene = app.build_scene(1.0, (PW, PH)).clone();
            reset_thread_policy();
            let glyphs: usize = scene
                .encoding()
                .resources
                .glyph_runs
                .iter()
                .map(|r| r.glyphs.len())
                .sum();
            for (name, aa) in [
                ("Msaa16", vello::AaConfig::Msaa16),
                ("Area", vello::AaConfig::Area),
            ] {
                let mut ms = Vec::new();
                for i in 0..23 {
                    let t = std::time::Instant::now();
                    renderer
                        .render_to_texture(
                            &device,
                            &queue,
                            &scene,
                            &view,
                            &vello::RenderParams {
                                base_color: vello::peniko::Color::WHITE,
                                width: PW,
                                height: PH,
                                antialiasing_method: aa,
                            },
                        )
                        .unwrap();
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    if i >= 3 {
                        ms.push(t.elapsed().as_secs_f64() * 1e3);
                    }
                }
                ms.sort_by(f64::total_cmp);
                eprintln!(
                    "text_aa_cost_2286 {scheme} darkening={darkening} {name}: median {:.2} ms \
                     (min {:.2}, max {:.2}), {glyphs} glyphs",
                    ms[ms.len() / 2],
                    ms[0],
                    ms[ms.len() - 1]
                );
            }
        }
    }
}

/// #2286 stage 3's cost probe: a 1280x800 page of 13px text drawn from the
/// atlas as grey masks (mask mode 1) and as subpixel masks (mode 2), the GPU
/// frame's median over 20 renders, under Area, plus the CPU cost of the
/// paint (`build_scene`, fresh atlas and warm) and the masks' bytes.
/// `--ignored --nocapture`; CI's `test-gpu` skips it by name.
#[test]
#[ignore]
fn lcd_cost_probe_2286() {
    use rinch_dom::paint::text_rasterization::{LcdFilter, SubpixelOrder, TextAntialias};
    const PW: u32 = 1280;
    const PH: u32 = 800;
    let (device, queue) = gpu();
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: PW,
            height: PH,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    for subpixel in [false, true] {
        let mut config = TextRasterization::default();
        config = if subpixel {
            config
                .antialias(TextAntialias::Subpixel)
                .subpixel_order(Some(SubpixelOrder::Rgb))
                .lcd_filter(Some(LcdFilter::Default))
        } else {
            config.antialias(TextAntialias::Grayscale)
        };
        set_thread_policy(config.resolve(&SystemTextSettings::default()));
        rinch_dom::paint::glyph_atlas::thread_atlas()
            .borrow_mut()
            .clear();
        let t = std::time::Instant::now();
        let mut app = text_app("#232629", "#ffffff", PW, PH);
        let cold = t.elapsed().as_secs_f64() * 1e3;
        let t = std::time::Instant::now();
        let scene = app.build_scene(1.0, (PW, PH)).clone();
        let warm = t.elapsed().as_secs_f64() * 1e3;
        let bytes = rinch_dom::paint::glyph_atlas::thread_atlas()
            .borrow()
            .bytes();
        reset_thread_policy();
        let mut ms = Vec::new();
        for i in 0..23 {
            let t = std::time::Instant::now();
            renderer
                .render_to_texture(
                    &device,
                    &queue,
                    &scene,
                    &view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::WHITE,
                        width: PW,
                        height: PH,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .unwrap();
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            if i >= 3 {
                ms.push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
        ms.sort_by(f64::total_cmp);
        eprintln!(
            "lcd_cost_2286 subpixel={subpixel}: GPU median {:.2} ms (min {:.2}, max {:.2}); \
             CPU first paint (mount + layout + rasterise) {cold:.1} ms, warm build_scene \
             {warm:.1} ms; atlas {} KiB",
            ms[ms.len() / 2],
            ms[0],
            ms[ms.len() - 1],
            bytes / 1024
        );
    }
}
