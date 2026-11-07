//! #2286 stage 3: subpixel (LCD) text on the GPU painter.
//!
//! The fork's mask mode 2 composites a per-channel coverage mask in the fine
//! shader: per channel where the pixel under it is opaque, grey elsewhere.
//! The destination is found per pixel, through the clip layers' backdrops
//! (`fine.wgsl`'s `lcd_composite`), so these pin, against the software
//! painter (which knows its pixmap exactly):
//!
//! - **parity**: subpixel text on an opaque page is the software painter's,
//!   to rounding, under Area and Msaa16, at 1x and 1.5x, with fringes in
//!   both (red against a fork that draws mode 2 as grey);
//! - **clips**: text inside one to six nested rounded clips on an opaque page
//!   is still per channel and still the software painter's; and where the
//!   opaque pixel is a backdrop past the blend stack's four registers, in its
//!   spill buffer, it is found there;
//! - **grey where it must be**: in an opacity group, over a transparent
//!   window (base colour transparent), and over a translucent background;
//!   per channel over an opaque box in the same transparent window;
//! - **BGR** is RGB with red and blue swapped.
//!
//! Comparisons are within two levels, not exact (the fine shader blends in
//! float, tiny-skia rounds per channel; Area's coverage can differ by one).
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_subpixel_text_2286 -- --ignored --test-threads=1`.
#![cfg(feature = "gpu")]

use std::cell::RefCell;
use std::rc::Rc;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::glyph_atlas::{GlyphAtlas, SharedAtlas};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::text_rasterization::{
    LcdFilter, ResolvedTextRasterization, StemDarkening, SubpixelOrder, SystemTextSettings,
    TextAntialias, TextRasterization, reset_thread_policy, set_thread_policy,
};
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 320;
const H: u32 = 120;

const INTER: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/Inter-Regular.ttf");

const TEXT: &str = "Sphinx of black quartz, judge my vow.";

fn policy(order: SubpixelOrder) -> ResolvedTextRasterization {
    TextRasterization::default()
        .antialias(TextAntialias::Subpixel)
        .subpixel_order(Some(order))
        .lcd_filter(Some(LcdFilter::Default))
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default())
}

/// `html` in a body of Inter 13px/18px, with `body_css`. The fractional
/// padding puts glyphs off the whole pixel.
fn doc(body_css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    doc.load_css(&format!(
        "html {{ margin: 0; }} body {{ margin: 0; padding: 6px 3.3px; \
         font: 13px/18px ProbeFace; color: #232629; {body_css} }} p {{ margin: 0; }}"
    ));
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

fn private_atlas() -> SharedAtlas {
    Rc::new(RefCell::new(GlyphAtlas::default()))
}

fn paint_vello(doc: &mut RinchDocument, scale: f64, atlas: &SharedAtlas) -> VelloPainter {
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
    painter
}

/// The software painter's pixels over `base` (premultiplied RGBA; the GPU
/// render's base colour).
fn paint_skia(
    doc: &mut RinchDocument,
    scale: f64,
    atlas: &SharedAtlas,
    base: [u8; 4],
) -> Vec<[u8; 4]> {
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.set_glyph_atlas(atlas.clone());
    for px in painter.pixels_mut().as_chunks_mut::<4>().0 {
        *px = base;
    }
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

/// The GPU target's straight RGBA premultiplied, to compare with the
/// software painter's: unpremultiplied, a faint pixel's colour carries its
/// rounding times `255 / alpha`.
fn premultiply(p: [u8; 4]) -> [u8; 4] {
    let c = |v: u8| ((v as u16 * p[3] as u16 + 127) / 255) as u8;
    [c(p[0]), c(p[1]), c(p[2]), p[3]]
}

/// Both painters' pixels for `html` under `order`, at `scale`, over a white
/// (or, `transparent`, a transparent) base, the GPU under `aa`.
fn both(
    gpu: &mut Gpu,
    body_css: &str,
    html: &str,
    order: SubpixelOrder,
    scale: f64,
    transparent: bool,
    aa: vello::AaConfig,
) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
    both_under(gpu, body_css, html, policy(order), scale, transparent, aa)
}

fn grey_policy() -> ResolvedTextRasterization {
    TextRasterization::default()
        .antialias(TextAntialias::Grayscale)
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default())
}

/// [`both`] under any policy.
fn both_under(
    gpu: &mut Gpu,
    body_css: &str,
    html: &str,
    policy: ResolvedTextRasterization,
    scale: f64,
    transparent: bool,
    aa: vello::AaConfig,
) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
    set_thread_policy(policy);
    let atlas = private_atlas();
    let mut d = doc(body_css, html);
    let painter = paint_vello(&mut d, scale, &atlas);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let (base, base_px) = if transparent {
        (peniko::Color::TRANSPARENT, [0, 0, 0, 0])
    } else {
        (peniko::Color::WHITE, [255; 4])
    };
    let g = gpu
        .render(painter.scene(), pw, ph, base, aa)
        .into_iter()
        .map(premultiply)
        .collect();
    let c = paint_skia(&mut d, scale, &atlas, base_px);
    reset_thread_policy();
    (g, c)
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

    fn render(
        &mut self,
        scene: &vello::Scene,
        pw: u32,
        ph: u32,
        base_color: peniko::Color,
        aa: vello::AaConfig,
    ) -> Vec<[u8; 4]> {
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
                        base_color,
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

/// Pixels whose colour channels differ by more than `tol` (alpha too), and
/// the largest difference.
fn differ(a: &[[u8; 4]], b: &[[u8; 4]], tol: u8) -> (usize, u8) {
    let mut n = 0;
    let mut worst = 0;
    for (p, q) in a.iter().zip(b) {
        let d = (0..4).map(|c| p[c].abs_diff(q[c])).max().unwrap();
        worst = worst.max(d);
        if d > tol {
            n += 1;
        }
    }
    (n, worst)
}

/// Opaque pixels whose colour channels differ by at least 24: fringes.
fn fringed(px: &[[u8; 4]]) -> usize {
    px.iter()
        .filter(|p| {
            let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            p[3] > 0 && hi - lo >= 24
        })
        .count()
}

/// Rows `y0..y1` of a `W`-wide (at 1x) image.
fn rows(px: &[[u8; 4]], y0: u32, y1: u32) -> &[[u8; 4]] {
    &px[(y0 * W) as usize..(y1 * W) as usize]
}

/// Subpixel text on an opaque page is the software painter's, with fringes
/// in both, under Area and Msaa16, at 1x and 1.5x: dark on light, light on
/// dark, translucent colour. Red against mode 2 drawn as grey (no fringes on
/// the GPU, and tens of levels apart at every fringe).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_subpixel_text_matches_the_software_painter() {
    let mut gpu = Gpu::new();
    let html = format!(
        "<p>{TEXT}</p>\
         <p style='background: #202326; color: #fcfcfc'>{TEXT}</p>\
         <p style='color: rgba(200, 40, 20, 0.8); letter-spacing: 0.37px'>{TEXT}</p>"
    );
    for aa in [vello::AaConfig::Area, vello::AaConfig::Msaa16] {
        for scale in [1.0, 1.5] {
            let (g, c) = both(&mut gpu, "", &html, SubpixelOrder::Rgb, scale, false, aa);
            let (fg, fc) = (fringed(&g), fringed(&c));
            assert!(fc > 300, "{aa:?} {scale}x: software fringes {fc}");
            assert!(fg > 300, "{aa:?} {scale}x: GPU fringes {fg}");
            let (n, worst) = differ(&g, &c, 2);
            assert_eq!(
                n, 0,
                "{aa:?} {scale}x: {n} pixels differ by more than 2 (worst {worst})"
            );
        }
    }
}

/// The clip fixtures' body: the clips' edges on whole pixels. The painters
/// differ at the edge pixels of nested clips there (#2321), grey text or not,
/// and an LCD fringe spilling into that column showed it. The text is put
/// off the whole pixel inside instead.
const CLIP_BODY: &str = "background: #fff; padding-left: 3px;";

/// `depth` nested rounded clips around `inner`, each a little narrower; the
/// clip at `opaque_at` (1-based) has a white background.
fn nested_clips(depth: usize, opaque_at: Option<usize>, inner: &str) -> String {
    let mut html = String::new();
    for i in 0..depth {
        let bg = if opaque_at == Some(i + 1) {
            "background: #fff; "
        } else {
            ""
        };
        html.push_str(&format!(
            "<div style='{bg}overflow: hidden; border-radius: 5px; padding: 0 {}px 1px 0'>",
            (depth - i) as f32 * 1.5
        ));
    }
    html.push_str(inner);
    for _ in 0..depth {
        html.push_str("</div>");
    }
    html
}

/// Text inside one, two, four, five and six nested rounded clips on an
/// opaque page is per channel and the software painter's, within two levels:
/// the shader composites the text's layer over each clip's backdrop until the
/// page (in the first register) makes it opaque.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_subpixel_text_inside_clips() {
    let mut gpu = Gpu::new();
    let mut report = Vec::new();
    for depth in [1usize, 2, 4, 5, 6] {
        let html = nested_clips(
            depth,
            None,
            &format!("<p style='padding-left: 0.3px'>{TEXT}</p>"),
        );
        let (g, c) = both(
            &mut gpu,
            CLIP_BODY,
            &html,
            SubpixelOrder::Rgb,
            1.0,
            false,
            vello::AaConfig::Area,
        );
        let (fg, fc) = (fringed(&g), fringed(&c));
        let (n, worst) = differ(&g, &c, 2);
        report.push((depth, fg, fc, n, worst));
    }
    assert!(
        report
            .iter()
            .all(|&(_, fg, fc, n, _)| fg > 150 && fc > 150 && n == 0),
        "(depth, GPU fringes, software fringes, pixels past 2 levels, worst): {report:?}"
    );
}

/// The opaque pixel is in the blend stack's spill buffer: in a transparent
/// window, the white background belongs to the fifth of six nested clips, so
/// the text's layer (inside the sixth) is opaque only over the backdrop the
/// sixth clip saved at depth five, past the four registers. Per channel, and
/// the software painter's. Red against a spill index that drops the per-depth
/// stride (`* TILE_WIDTH * TILE_HEIGHT` in `lcd_composite`): that reads depth
/// four's backdrop, transparent, and the text falls back to grey.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_subpixel_text_finds_the_page_in_the_spill() {
    let mut gpu = Gpu::new();
    let mut report = Vec::new();
    for (depth, opaque_at) in [(6usize, 5usize), (7, 5), (7, 6)] {
        let html = nested_clips(
            depth,
            Some(opaque_at),
            &format!("<p style='padding-left: 0.3px'>{TEXT}</p>"),
        );
        let (g, c) = both(
            &mut gpu,
            "padding-left: 3px;",
            &html,
            SubpixelOrder::Rgb,
            1.0,
            true,
            vello::AaConfig::Area,
        );
        let (fg, fc) = (fringed(&g), fringed(&c));
        let (n, worst) = differ(&g, &c, 2);
        report.push((depth, opaque_at, fg, fc, n, worst));
    }
    assert!(
        report
            .iter()
            .all(|&(_, _, fg, fc, n, _)| fg > 150 && fc > 150 && n == 0),
        "(depth, opaque clip, GPU fringes, software fringes, pixels past 2 levels, \
         worst): {report:?}"
    );
}

/// Grey where the destination is not opaque or is a layer: an opacity group
/// on an opaque page; a transparent window, with nothing under the text; a
/// translucent background in it. Per channel over an opaque box in the same
/// transparent window. The GPU and the software painter agree throughout.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_subpixel_text_falls_back_to_grey() {
    let mut gpu = Gpu::new();
    // Line boxes: 6..24, 24..42, 42..60, 60..78.
    let (g, c) = both(
        &mut gpu,
        "background: #fff;",
        &format!("<p style='opacity: 0.9'>{TEXT}</p><p>{TEXT}</p>"),
        SubpixelOrder::Rgb,
        1.0,
        false,
        vello::AaConfig::Area,
    );
    assert_eq!(
        fringed(rows(&g, 6, 24)),
        0,
        "GPU: grey in the opacity group"
    );
    assert!(
        fringed(rows(&g, 24, 42)) > 150,
        "GPU: per channel beside it"
    );
    assert_eq!(differ(&g, &c, 2).0, 0, "{:?}", differ(&g, &c, 2));

    let (g, c) = both(
        &mut gpu,
        "",
        &format!(
            "<p>{TEXT}</p><p style='background: rgba(255,255,255,0.6)'>{TEXT}</p>\
             <p style='background: #fff'>{TEXT}</p>"
        ),
        SubpixelOrder::Rgb,
        1.0,
        true,
        vello::AaConfig::Area,
    );
    let ink = rows(&g, 6, 24).iter().filter(|p| p[3] > 0).count();
    assert!(ink > 300, "the bare text is painted: {ink}");
    assert_eq!(fringed(rows(&g, 6, 24)), 0, "GPU: grey over transparency");
    assert_eq!(fringed(rows(&g, 24, 42)), 0, "GPU: grey over translucency");
    assert!(
        fringed(rows(&g, 42, 60)) > 150,
        "GPU: per channel over the box"
    );
    let (n, worst) = differ(&g, &c, 2);
    assert_eq!(
        n, 0,
        "transparent window: {n} pixels differ (worst {worst})"
    );
}

/// BGR is RGB with red and blue swapped on the GPU too (black on white).
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_bgr_swaps_red_and_blue() {
    let mut gpu = Gpu::new();
    let html = format!("<p style='color: #000'>{TEXT}</p>");
    let (rgb, _) = both(
        &mut gpu,
        "background: #fff;",
        &html,
        SubpixelOrder::Rgb,
        1.0,
        false,
        vello::AaConfig::Area,
    );
    let (bgr, _) = both(
        &mut gpu,
        "background: #fff;",
        &html,
        SubpixelOrder::Bgr,
        1.0,
        false,
        vello::AaConfig::Area,
    );
    assert!(fringed(&rgb) > 150);
    let swapped: Vec<[u8; 4]> = rgb.iter().map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let (n, worst) = differ(&bgr, &swapped, 1);
    assert_eq!(n, 0, "{n} pixels differ from the swap (worst {worst})");
}

/// TEMPORARY probe for #2321 (not to commit).
#[test]
#[ignore = "probe"]
fn probe_2321_nested_clip_edges() {
    let mut gpu = Gpu::new();
    for scale in [1.0f64, 1.25] {
        for radius in ["0", "5px"] {
            for pad in ["3px", "3.3px"] {
                let mut row = Vec::new();
                for depth in 1..=6usize {
                    let mut html = String::new();
                    for _ in 0..depth {
                        html.push_str(&format!(
                            "<div style='overflow: hidden; border-radius: {radius}'>"
                        ));
                    }
                    html.push_str("<div style='height: 40px; background: #000'></div>");
                    for _ in 0..depth {
                        html.push_str("</div>");
                    }
                    let (g, c) = both_under(
                        &mut gpu,
                        &format!("background: #fff; padding: 6px {pad};"),
                        &html,
                        grey_policy(),
                        scale,
                        false,
                        vello::AaConfig::Area,
                    );
                    let pw = (W as f64 * scale) as u32;
                    let y = (26.0 * scale) as u32;
                    let x0 = (3.0 * scale) as u32;
                    let at = |px: &[[u8; 4]], x: u32| px[(y * pw + x) as usize][0];
                    let yc = (6.5 * scale) as u32 + 1;
                    let corner = |px: &[[u8; 4]]| px[(yc * pw + x0 + 1) as usize][0];
                    row.push((
                        depth,
                        (at(&g, x0), at(&c, x0)),
                        (at(&g, x0 + 1), at(&c, x0 + 1)),
                        (corner(&g), corner(&c)),
                    ));
                }
                eprintln!(
                    "PROBE scale {scale} radius {radius} pad {pad}: (depth, (gpu,cpu) at x0, at x0+1, corner) {row:?}"
                );
            }
        }
    }
}
