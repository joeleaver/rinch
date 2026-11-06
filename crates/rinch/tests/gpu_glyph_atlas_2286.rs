//! #2286 stage 2: the glyph-mask atlas on the GPU painter.
//!
//! Text painted by `VelloPainter` and rendered by Vello (lavapipe in CI) is
//! drawn from the same swash masks as the software painter, through the
//! fork's tinted alpha-mask images. These pin:
//!
//! - **parity**: the GPU's glyph pixels are the software painter's, to
//!   rounding, at 1x and at a HiDPI scale, over light and dark backgrounds
//!   (red with the atlas off, where Vello draws outlines);
//! - **one mask, every colour**: the same text in two colours rasterises each
//!   (glyph, bucket) once and paints each in its own colour;
//! - **the fallbacks**: a colour glyph, a device size past the atlas's limit,
//!   a rotation and a synthetic italic are drawn as outlines, inked and
//!   counted; a CSS `scale()` is rasterised again at the device size;
//! - **the GPU's lifetime**: a renderer on a new device draws the same masks
//!   (they are CPU blobs; Vello's atlas is the renderer's);
//! - **eviction**: an atlas far too small for the page still paints it right,
//!   frame after frame.
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_glyph_atlas_2286 -- --ignored --test-threads=1`.
#![cfg(feature = "gpu")]

use std::cell::RefCell;
use std::rc::Rc;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::glyph_atlas::{GlyphAtlas, SharedAtlas};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::vello_painter::{GlyphStats, VelloPainter};

const W: u32 = 320;
const H: u32 = 160;

const INTER: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/Inter-Regular.ttf");
const EMOJI: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/NotoColorEmoji-U1F600.ttf");

/// Text in the bundled Inter (registered under its own name, so the host's
/// fonts cannot answer) with declared line boxes. The fractional
/// `padding-left`s and `letter-spacing` put glyphs in every sub-pixel bucket.
const PAGE: &str = r#"
<div style="padding-left: 3.3px; font: 13px/18px ProbeFace; color: #232629">Sphinx of black quartz, judge my vow.</div>
<div style="padding-left: 2.6px; font: 16px/22px ProbeFace; color: #111; letter-spacing: 0.37px">Pack my box with five dozen jugs</div>
<div style="background: #202326; padding-left: 1.9px; font: 13px/18px ProbeFace; color: #fcfcfc">The five boxing wizards jump quickly</div>
<div style="padding-left: 0.45px; font: 11px/16px ProbeFace; color: rgba(200, 40, 20, 0.8)">Translucent 0123456789 glyphs</div>
"#;

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    for (bytes, name) in [(INTER, "ProbeFace"), (EMOJI, "ProbeEmoji")] {
        let registered = doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(bytes)),
            Some(FontInfoOverride {
                family_name: Some(name),
                ..Default::default()
            }),
        );
        assert_eq!(registered.len(), 1, "{name}: one file, one family");
    }
    doc.load_css("html, body { margin: 0; background: #fff; }");
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn private_atlas(budget: usize) -> SharedAtlas {
    Rc::new(RefCell::new(GlyphAtlas::with_budget(budget)))
}

fn paint_vello(
    doc: &mut RinchDocument,
    scale: f64,
    atlas: &SharedAtlas,
) -> (VelloPainter, GlyphStats) {
    let mut painter = VelloPainter::new();
    painter.set_glyph_atlas(atlas.clone());
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let stats = painter.take_glyph_stats();
    (painter, stats)
}

fn paint_skia(doc: &mut RinchDocument, scale: f64, atlas: &SharedAtlas) -> Vec<[u8; 4]> {
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.set_glyph_atlas(atlas.clone());
    // The window background the GPU render's base colour gives.
    painter.pixels_mut().fill(255);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
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

    fn render(&mut self, scene: &vello::Scene, pw: u32, ph: u32) -> Vec<[u8; 4]> {
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
                        antialiasing_method: vello::AaConfig::Msaa16,
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

/// Pixels that are not the page's backgrounds: text ink.
fn inked(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| {
            let rgb = [p[0], p[1], p[2]];
            rgb != [255, 255, 255] && rgb != [0x20, 0x23, 0x26]
        })
        .count()
}

/// The GPU's text is the software painter's, glyph for glyph: both draw the
/// atlas's swash masks at the same whole pixels and buckets, and composite
/// coverage in sRGB-encoded space. Off by at most a level or two of rounding
/// (the fine shader blends in float, tiny-skia in 8 bits). At 1x and at
/// 1.5x, where the masks are rasterised at the device size.
///
/// Red with the atlas off (`draw_atlas_glyphs` returning `false`): Vello's
/// outlines under Msaa16 differ from swash's masks by tens of levels at
/// glyph edges (design-2286 §1: MAE 0.049 of full scale).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_atlas_glyphs_match_the_software_painter() {
    let mut gpu = Gpu::new();
    for scale in [1.0, 1.5] {
        let atlas = private_atlas(usize::MAX);
        let mut d = doc(PAGE);
        let (painter, stats) = paint_vello(&mut d, scale, &atlas);
        let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
        let gpu_px = gpu.render(painter.scene(), pw, ph);
        let cpu_px = paint_skia(&mut d, scale, &atlas);
        let ink = inked(&cpu_px);
        assert!(
            ink > 2000,
            "{scale}x: the page has text ({ink} inked pixels)"
        );
        let (n, worst) = differ(&gpu_px, &cpu_px, 2);
        assert_eq!(
            n, 0,
            "{scale}x: {n} pixels differ by more than 2 levels (worst {worst}) \
             between the GPU and the software painter"
        );
        assert_eq!(
            stats.outline_fallbacks, 0,
            "{scale}x: every glyph from the atlas"
        );
        assert!(
            stats.atlas.misses > 0 && stats.atlas.uploads > 0,
            "{stats:?}"
        );
    }
}

/// One mask serves every colour: the same words in red and in blue
/// rasterise each (glyph, bucket) once, upload it once, and paint each line
/// in its own colour.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_one_mask_serves_every_colour() {
    let line = |color: &str| {
        format!(r#"<div style="font: 16px/22px ProbeFace; color: {color}">mmmm</div>"#)
    };
    let mut d = doc(&format!("{}{}", line("#f00"), line("#00f")));
    let atlas = private_atlas(usize::MAX);
    let (painter, stats) = paint_vello(&mut d, 1.0, &atlas);
    // "mmmm" at 16px: four glyphs a line; the advances put them at up to
    // four buckets. The second line, at the same x, hits every one.
    assert_eq!(stats.atlas.hits + stats.atlas.misses, 8, "{stats:?}");
    assert!(
        stats.atlas.hits >= 4,
        "the blue line reuses the red masks: {stats:?}"
    );
    assert_eq!(stats.atlas.uploads, stats.atlas.misses, "{stats:?}");
    let px = Gpu::new().render(painter.scene(), W, H);
    let darkest = |y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| px[(y * W + x) as usize])
            .min_by_key(|p| p[0] as u32 + p[1] as u32 + p[2] as u32)
            .unwrap()
    };
    let red = darkest(0, 22);
    let blue = darkest(22, 44);
    assert_eq!([red[0], red[1], red[2]], [255, 0, 0], "a solid red stem");
    assert_eq!(
        [blue[0], blue[1], blue[2]],
        [0, 0, 255],
        "a solid blue stem"
    );
}

/// What the atlas does not draw is drawn as outlines, inked, and counted:
/// a colour glyph (the bundled emoji's bitmap strike: Vello draws it as an
/// image itself), a size past `ATLAS_MAX_FONT_PX`, a rotation, a synthetic
/// italic. A CSS `scale(2)` is not a fallback: the masks are made at the
/// device size, twice as wide.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_fallbacks_draw_outlines() {
    let mut gpu = Gpu::new();
    let cases: [(&str, &str, bool); 5] = [
        (
            "emoji",
            r#"<div style="font: 20px/28px ProbeEmoji">&#x1F600;</div>"#,
            true,
        ),
        (
            "80px",
            r#"<div style="font: 80px/96px ProbeFace">Hg</div>"#,
            true,
        ),
        (
            "rotated",
            r#"<div style="font: 16px/22px ProbeFace; transform: rotate(7deg); width: 200px">rotated</div>"#,
            true,
        ),
        (
            "italic",
            r#"<div style="font: italic 16px/22px ProbeFace">synthetic</div>"#,
            true,
        ),
        (
            "scale(2)",
            r#"<div style="font: 13px/18px ProbeFace; transform: scale(2); transform-origin: 0 0">scaled</div>"#,
            false,
        ),
    ];
    for (name, html, fallback) in cases {
        let atlas = private_atlas(usize::MAX);
        let mut d = doc(html);
        let (painter, stats) = paint_vello(&mut d, 1.0, &atlas);
        if fallback {
            assert!(
                stats.outline_fallbacks > 0 && stats.atlas.uploads == 0,
                "{name}: drawn as outlines: {stats:?}"
            );
        } else {
            assert_eq!(stats.outline_fallbacks, 0, "{name}: {stats:?}");
            assert!(stats.atlas.uploads > 0, "{name}: {stats:?}");
        }
        let px = gpu.render(painter.scene(), W, H);
        let ink = inked(&px);
        assert!(ink > 40, "{name}: inked ({ink} pixels)");
        if name == "emoji" {
            let coloured = px
                .iter()
                .filter(|p| p[0].abs_diff(p[1]) > 40 || p[1].abs_diff(p[2]) > 40)
                .count();
            assert!(coloured > 40, "the emoji is drawn in colour ({coloured})");
        }
    }
    // The scaled text's masks are the 26px ones: about what the same word
    // set at 26px with no transform keeps (the buckets may differ), and well
    // over twice the 13px word's (four times the pixels, less the per-entry
    // overhead).
    let bytes = |html: &str| {
        let atlas = private_atlas(usize::MAX);
        let mut d = doc(html);
        let _ = paint_vello(&mut d, 1.0, &atlas);
        atlas.borrow().bytes()
    };
    let scaled = bytes(cases[4].1);
    let small = bytes(r#"<div style="font: 13px/18px ProbeFace">scaled</div>"#);
    let big = bytes(r#"<div style="font: 26px/36px ProbeFace">scaled</div>"#);
    assert!(
        scaled * 10 > big * 9 && scaled * 9 < big * 10,
        "scale(2) keeps 26px masks: {scaled} vs {big} bytes"
    );
    assert!(
        scaled * 5 > small * 11,
        "more than 2.2x the 13px masks: {scaled} vs {small}"
    );
}

/// The app's escape hatch: `TextRasterization::gpu_glyph_outlines(true)`
/// draws every glyph as an outline, nothing from the atlas.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_outlines_on_request() {
    use rinch_dom::paint::text_rasterization::{
        SystemTextSettings, TextRasterization, reset_thread_policy, set_thread_policy,
    };
    set_thread_policy(
        TextRasterization::default()
            .gpu_glyph_outlines(true)
            .resolve(&SystemTextSettings::default()),
    );
    let atlas = private_atlas(usize::MAX);
    let mut d = doc(PAGE);
    let (painter, stats) = paint_vello(&mut d, 1.0, &atlas);
    reset_thread_policy();
    assert_eq!(stats.atlas, Default::default(), "{stats:?}");
    assert!(stats.outline_fallbacks > 100, "{stats:?}");
    let px = Gpu::new().render(painter.scene(), W, H);
    assert!(inked(&px) > 2000, "outlines are drawn");
}

/// The masks outlive the GPU: a renderer on a new device (a device lost and
/// made again) draws the same scene to the same pixels, and painting it
/// again rasterises and uploads nothing new — the atlas holds CPU blobs, and
/// each renderer's Vello atlas uploads them when it first draws them.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_masks_survive_a_new_renderer() {
    let atlas = private_atlas(usize::MAX);
    let mut d = doc(PAGE);
    let (painter, first) = paint_vello(&mut d, 1.0, &atlas);
    let a = Gpu::new().render(painter.scene(), W, H);
    // A new device and renderer: what a lost device leads to.
    let b = Gpu::new().render(painter.scene(), W, H);
    assert_eq!(
        differ(&a, &b, 1).0,
        0,
        "the same pixels from a new renderer"
    );
    let (painter2, again) = paint_vello(&mut d, 1.0, &atlas);
    assert_eq!(
        (again.atlas.misses, again.atlas.uploads),
        (0, 0),
        "{again:?}"
    );
    assert_eq!(again.atlas.hits, first.atlas.hits + first.atlas.misses);
    let c = Gpu::new().render(painter2.scene(), W, H);
    assert_eq!(differ(&a, &c, 1).0, 0, "the repaint from a third renderer");
}

/// An atlas a fraction of the page's masks evicts on every frame and still
/// paints the page exactly as an unbounded one does, frame after frame, on
/// one renderer (whose own image atlas then holds blobs the glyph atlas has
/// dropped, and gets new ones for the same glyphs).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_eviction_keeps_drawing() {
    let mut gpu = Gpu::new();
    let mut d = doc(PAGE);
    let full = private_atlas(usize::MAX);
    let (painter, all) = paint_vello(&mut d, 1.0, &full);
    let want = gpu.render(painter.scene(), W, H);
    let need = full.borrow().bytes();
    let small = private_atlas(need / 8);
    let mut evicted = 0;
    for frame in 0..3 {
        let (painter, stats) = paint_vello(&mut d, 1.0, &small);
        evicted += stats.atlas.evictions;
        assert!(
            small.borrow().bytes() <= need / 8,
            "frame {frame}: within budget"
        );
        let got = gpu.render(painter.scene(), W, H);
        let (n, worst) = differ(&got, &want, 1);
        assert_eq!(n, 0, "frame {frame}: {n} pixels differ (worst {worst})");
    }
    assert!(evicted > 0, "the small atlas evicted ({all:?})");
}
