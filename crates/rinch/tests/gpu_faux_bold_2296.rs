//! #2296: synthetic (faux) bold on the GPU painter, rendered by Vello
//! (lavapipe in CI).
//!
//! A family with no bold face set in `font-weight: bold` is drawn from its
//! regular face, emboldened as Chrome emboldens it: Skia's stroke `size × k`
//! wide, `k` 1/24 of the em up to 9px and 1/32 from 36px (measured in Chrome
//! 156; `rinch_dom::paint::text_rasterization::synthetic_bold_px`). These pin:
//!
//! - **the atlas**: faux-bold text, dark and light (darkened as well), with a
//!   hard `text-shadow`, is the software painter's pixels, to rounding (both
//!   draw the same emboldened swash masks);
//! - **the outlines**: with the atlas off, and past the atlas's size limit,
//!   Vello's own expansion (`font_embolden`) grows each stem by Chrome's
//!   amount, centred, as the atlas's masks do.
//!
//! Red before #2296: no painter emboldened, so the bold rows were the
//! regular rows.
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_faux_bold_2296 -- --ignored --test-threads=1`.
#![cfg(feature = "gpu")]

use std::cell::RefCell;
use std::rc::Rc;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::glyph_atlas::{GlyphAtlas, SharedAtlas};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::text_rasterization::{
    StemDarkening, SystemTextSettings, TextHinting, TextRasterization, reset_thread_policy,
    set_thread_policy,
};
use rinch_dom::paint::vello_painter::{GlyphStats, VelloPainter};

const W: u32 = 480;
const H: u32 = 240;

/// Inter's regular face, registered alone: the family has no bold.
const INTER: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/Inter-Regular.ttf");

/// Regular and faux-bold text, dark and light, at fractional offsets (every
/// sub-pixel bucket), one row with a hard shadow.
const PAGE: &str = r#"
<div style="padding-left: 3.3px; font: 13px/20px ProbeFace; color: #232629">Sphinx of black quartz, judge my vow.</div>
<div style="padding-left: 3.3px; font: bold 13px/20px ProbeFace; color: #232629">Sphinx of black quartz, judge my vow.</div>
<div style="padding-left: 2.6px; font: bold 16px/24px ProbeFace; color: #111; letter-spacing: 0.37px">Pack my box with five dozen jugs</div>
<div style="background: #202326; padding-left: 1.9px; font: bold 13px/20px ProbeFace; color: #fcfcfc">The five boxing wizards jump quickly</div>
<div style="padding-left: 0.45px; font: bold 20px/30px ProbeFace; color: #222; text-shadow: 2px 2px 0 #c03020">Shadowed 0123456789</div>
<div style="padding-left: 1.2px; font: bold 40px/52px ProbeFace; color: #000">Hamburg</div>
"#;

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one face");
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn private_atlas() -> SharedAtlas {
    Rc::new(RefCell::new(GlyphAtlas::with_budget(usize::MAX)))
}

fn paint_vello(doc: &mut RinchDocument, atlas: Option<&SharedAtlas>) -> (VelloPainter, GlyphStats) {
    let mut painter = VelloPainter::new();
    match atlas {
        Some(a) => painter.set_glyph_atlas(a.clone()),
        None => painter.set_glyph_atlas_enabled(false),
    }
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let stats = painter.take_glyph_stats();
    (painter, stats)
}

fn paint_skia(doc: &mut RinchDocument, atlas: &SharedAtlas) -> Vec<[u8; 4]> {
    let mut painter = TinySkiaPainter::new(W, H);
    painter.set_glyph_atlas(atlas.clone());
    painter.pixels_mut().fill(255);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
}

impl Gpu {
    fn new() -> Self {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("a GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .unwrap();
        Self {
            device,
            queue,
            renderer,
        }
    }

    fn render(&mut self, scene: &vello::Scene, aa: vello::AaConfig) -> Vec<[u8; 4]> {
        let (pw, ph) = (W, H);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // A frame whose bump buffers overflowed is reported and drawn again
        // (docs/internals/app-and-windows.md "Bump buffers").
        for _ in 0..4 {
            self.renderer
                .render_to_texture(
                    &self.device,
                    &self.queue,
                    scene,
                    &view,
                    &vello::RenderParams {
                        base_color: peniko::Color::WHITE,
                        width: pw,
                        height: ph,
                        antialiasing_method: aa,
                    },
                )
                .unwrap();
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if !matches!(
                self.renderer.check_frames(&self.device),
                vello::FrameCheck::Lost
            ) {
                break;
            }
        }
        let row = (pw * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * ph) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(ph),
                },
            },
            wgpu::Extent3d {
                width: pw,
                height: ph,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .expect("the buffer is mapped");
        (0..ph)
            .flat_map(|y| {
                let data = &data;
                (0..pw).map(move |x| {
                    let i = (y * row + x * 4) as usize;
                    [data[i], data[i + 1], data[i + 2], data[i + 3]]
                })
            })
            .collect()
    }
}

/// Pixels whose colour channels differ by more than `tol`, and the largest
/// difference.
fn differ(a: &[[u8; 4]], b: &[[u8; 4]], tol: u8) -> (usize, u8) {
    let mut n = 0;
    let mut worst = 0;
    for (p, q) in a.iter().zip(b) {
        let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
        worst = worst.max(d);
        if d > tol {
            n += 1;
        }
    }
    (n, worst)
}

/// Ink (`0` white, `1` black) of the rows `y0..y1`, row-major.
fn ink_rows(px: &[[u8; 4]], y0: u32, y1: u32) -> Vec<f64> {
    (y0..y1)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| 1.0 - px[(y * W + x) as usize][0] as f64 / 255.0)
        .collect()
}

/// Mean width of the ten stems of `llllllllll` in a band, over five rows at
/// their middle, and the ink's centre.
fn stems(c: &[f64], rows: u32) -> (f64, (f64, f64)) {
    let at = |x: u32, y: u32| c[(y * W + x) as usize];
    let inked: Vec<u32> = (0..rows)
        .filter(|&y| (0..W).map(|x| at(x, y)).sum::<f64>() > 0.5)
        .collect();
    let mid = (inked[0] + inked[inked.len() - 1]) / 2;
    let width = (mid - 2..=mid + 2)
        .map(|y| (0..W).map(|x| at(x, y)).sum::<f64>())
        .sum::<f64>()
        / 50.0;
    let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
    for y in 0..rows {
        for x in 0..W {
            let v = at(x, y);
            sx += v * x as f64;
            sy += v * y as f64;
            s += v;
        }
    }
    (width, (sx / s, sy / s))
}

/// Chrome's total stem growth at `size` CSS px (the module doc).
fn chrome_growth(size: f64) -> f64 {
    let k = 1.0 / 24.0 + ((size - 9.0) / 27.0).clamp(0.0, 1.0) * (1.0 / 32.0 - 1.0 / 24.0);
    size * k
}

/// Faux-bold text on the GPU's atlas is the software painter's, glyph for
/// glyph, darkened light text and a hard shadow included: both draw the same
/// emboldened masks. And it is bold: the bold row carries more ink than the
/// same text regular.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_faux_bold_atlas_matches_the_software_painter() {
    let mut gpu = Gpu::new();
    let atlas = private_atlas();
    let mut d = doc(PAGE);
    let (painter, stats) = paint_vello(&mut d, Some(&atlas));
    let gpu_px = gpu.render(painter.scene(), vello::AaConfig::Msaa16);
    let cpu_px = paint_skia(&mut d, &atlas);
    let (n, worst) = differ(&gpu_px, &cpu_px, 2);
    assert_eq!(
        n, 0,
        "{n} pixels differ by more than 2 levels (worst {worst}) between the GPU and the \
         software painter"
    );
    assert_eq!(stats.outline_fallbacks, 0, "every glyph from the atlas");
    let synthesis = atlas.borrow().run_synthesis();
    assert!(
        synthesis.iter().any(|s| s.0 > 0.0) && synthesis.iter().any(|s| s.0 == 0.0),
        "regular and faux-bold runs keyed apart: {synthesis:?}"
    );
    let ink = |y0| ink_rows(&gpu_px, y0, y0 + 20).iter().sum::<f64>();
    let (regular, bold) = (ink(0), ink(20));
    assert!(
        bold > regular * 1.12,
        "the bold row carries {bold:.1}, the regular {regular:.1}"
    );
}

/// Vello's outlines grow a faux-bold stem by Chrome's amount, centred, as the
/// atlas's masks do: with the atlas off at 48px, and at 80px, past the
/// atlas's size limit, where outlines are the GPU painter's own path. Under
/// area coverage, so the edges are exact enough to measure.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_faux_bold_outlines_grow_like_chrome() {
    let mut gpu = Gpu::new();
    set_thread_policy(
        TextRasterization::default()
            .hinting(TextHinting::None)
            .stem_darkening(StemDarkening::Off)
            .resolve(&SystemTextSettings::default()),
    );
    for (size, atlas_on) in [(48.0, false), (80.0, true)] {
        let band = (size * 1.5) as u32;
        let html = format!(
            r#"<div style="padding-left: 10.3px; font: {size}px/{band}px ProbeFace">llllllllll</div>
<div style="padding-left: 10.3px; font: bold {size}px/{band}px ProbeFace">llllllllll</div>"#
        );
        let mut d = doc(&html);
        let atlas = private_atlas();
        let (painter, stats) = paint_vello(&mut d, atlas_on.then_some(&atlas));
        assert!(
            stats.outline_fallbacks > 0,
            "{size}px: outlines ({stats:?})"
        );
        let px = gpu.render(painter.scene(), vello::AaConfig::Area);
        let (w0, c0) = stems(&ink_rows(&px, 0, band), band);
        let (w1, c1) = stems(&ink_rows(&px, band, 2 * band), band);
        let growth = w1 - w0;
        let want = chrome_growth(size);
        assert!(
            (growth - want).abs() < 0.1,
            "{size}px: stems grew {growth:.3} px, Chrome {want:.3}"
        );
        assert!(
            (c1.0 - c0.0).abs() < 0.05 && (c1.1 - c0.1).abs() < 0.05,
            "{size}px: the ink's centre moved from {c0:?} to {c1:?}"
        );
    }
    reset_thread_policy();
}
