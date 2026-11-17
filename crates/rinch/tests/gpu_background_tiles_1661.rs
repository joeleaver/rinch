//! #1661, #1755 and #1753 on the GPU painter: repeating backgrounds painted
//! by `VelloPainter` and rendered by Vello.
//!
//! Vello's bilinear image arm applied `Extend::Repeat` to the sample point
//! and clamped afterwards, so the pixel on the boundary between two tiles
//! read one tile's edge texel where Chrome and tiny-skia blend the two
//! (#1661). To keep a sub-pixel `background-position` animation from pulsing
//! (#1755), the painter then placed a fractional tile on whole device pixels.
//! The fork (joeleaver/vello-fork, `rinch-patch`) wraps each tap, and the
//! painter keeps the fraction. A `url()` tile is sampled straight from the
//! image (#1753), one bilinear sample per device pixel.
//!
//! Chrome 155's pixels (`google-chrome --headless --disable-gpu
//! --force-device-scale-factor=1.5 --screenshot`, each box a 60x20 div at the
//! page's top left): Skia keeps a texel fraction to 1/16, so [`TOL`] is 16
//! levels. Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_background_tiles_1661 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::painter::Painter;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::vello_painter::VelloPainter;

const TOL: i32 = 16;
const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];
const W: [u8; 3] = [255, 255, 255];
const RB: &str = "linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

/// 10x10: left half red and right half blue (`lr.png`), or top half red and
/// bottom half blue (`tb.png`).
fn halves(vertical: bool) -> DecodedImage {
    let mut data = Vec::new();
    for y in 0..10 {
        for x in 0..10 {
            let first = if vertical { y < 5 } else { x < 5 };
            data.extend_from_slice(if first {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    DecodedImage::new(data, 10, 10)
}

/// One box per style, stacked 30px apart, painted into `painter` at
/// `scale`; the page is 100 x `30 * styles.len()` CSS px.
fn paint(painter: &mut dyn Painter, styles: &[&str], scale: f64) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    doc.tree
        .image_cache
        .insert_decoded("lr.png".into(), halves(false));
    doc.tree
        .image_cache
        .insert_decoded("tb.png".into(), halves(true));
    let body = doc.body();
    for (i, style) in styles.iter().enumerate() {
        let n = doc.create_element("div");
        doc.set_attribute(
            n,
            "style",
            &format!(
                "position: absolute; left: 0; top: {}px; width: 60px; height: 20px; {style}",
                30 * i
            ),
        );
        doc.append_child(body, n);
    }
    let h = 30.0 * styles.len() as f32;
    doc.resolve_layout(100.0, h);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        scale,
        (100.0, h),
        &mut doc.font_cx,
        &mut layout_cx,
    );
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
                antialiasing_support: vello::AaSupport::area_only(),
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

    /// The styles' boxes at `scale`, rendered: (width, RGBA rows).
    fn render(&mut self, styles: &[&str], scale: f64) -> (u32, Vec<[u8; 4]>) {
        let mut painter = VelloPainter::new();
        paint(&mut painter, styles, scale);
        let w = (100.0 * scale) as u32;
        let h = (30.0 * styles.len() as f64 * scale) as u32;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
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
        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                painter.scene(),
                &view,
                &vello::RenderParams {
                    base_color: peniko::Color::WHITE,
                    width: w,
                    height: h,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .unwrap();
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
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
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
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
        let px = (0..h)
            .flat_map(|y| {
                let data = &data;
                (0..w).map(move |x| {
                    let i = (y * row + x * 4) as usize;
                    [data[i], data[i + 1], data[i + 2], data[i + 3]]
                })
            })
            .collect();
        (w, px)
    }
}

/// The software painter's pixels for the same boxes.
fn software(styles: &[&str], scale: f64) -> (u32, Vec<[u8; 4]>) {
    let w = (100.0 * scale) as u32;
    let h = (30.0 * styles.len() as f64 * scale) as u32;
    let mut painter = TinySkiaPainter::new(w, h);
    paint(&mut painter, styles, scale);
    let px = painter
        .pixels()
        .chunks(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    (w, px)
}

fn off(got: [u8; 4], want: [u8; 3]) -> i32 {
    (0..3)
        .map(|k| (got[k] as i32 - want[k] as i32).abs())
        .max()
        .unwrap()
}

/// The Chrome cases of `rinch-dom/tests/background_image_tile_sampling_tests.rs`
/// at DPR 1.5, one box each (box `i`'s row `y` is device row `45 i + y`).
/// Pixels 0, 14 and 15 of the first box sit on tile seams; on `main` the GPU
/// read the nearest raster texel there (and everywhere), and with the
/// painter's whole-pixel step removed but Vello's clamp kept, the seam pixels
/// took one side.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_scaled_url_tiles_blend_across_their_seams_as_chrome() {
    let styles = [
        "background: url(lr.png) 0 0 / 10px 10px",
        "background: url(lr.png) 0.5px 0 / 10px 10px",
        "background: url(lr.png) 0.3px 0 / 13px 10px",
        "background: url(tb.png) 0 0 / 10px 10px repeat-x",
        "background: linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%) \
         0.5px 0 / 7px 10px",
    ];
    let mut want: Vec<((u32, u32), [u8; 3])> = Vec::new();
    let row = |b: u32, cells: &[(u32, [u8; 3])]| -> Vec<((u32, u32), [u8; 3])> {
        cells.iter().map(|&(x, c)| ((x, 45 * b + 10), c)).collect()
    };
    want.extend(row(
        0,
        &[
            (0, [207, 0, 47]),
            (6, R),
            (7, [127, 0, 127]),
            (8, B),
            (14, [31, 0, 223]),
            (15, [207, 0, 47]),
            (22, [127, 0, 127]),
            (29, [31, 0, 223]),
            (30, [207, 0, 47]),
        ],
    ));
    want.extend(row(
        1,
        &[
            (0, [79, 0, 175]),
            (7, R),
            (8, [95, 0, 159]),
            (9, B),
            (15, [79, 0, 175]),
            (23, [95, 0, 159]),
        ],
    ));
    want.extend(row(
        2,
        &[
            (0, [127, 0, 127]),
            (9, [223, 0, 31]),
            (10, [95, 0, 159]),
            (19, [63, 0, 191]),
            (20, [191, 0, 63]),
            (29, [159, 0, 95]),
            (30, [31, 0, 223]),
            (39, [127, 0, 127]),
        ],
    ));
    for x in [0, 7, 22] {
        for (y, c) in [(0, R), (6, R), (7, [128, 0, 128]), (8, B), (14, B), (15, W)] {
            want.push(((x, 135 + y), c));
        }
    }
    // #1757's gradient tile (10.5 device px, rasterised at 11).
    want.extend(row(
        4,
        &[
            (0, [61, 0, 194]),
            (5, R),
            (6, [121, 0, 134]),
            (7, B),
            (11, [194, 0, 61]),
            (16, R),
            (17, B),
        ],
    ));
    let mut gpu = Gpu::new();
    let (gw, g) = gpu.render(&styles, 1.5);
    let (sw, s) = software(&styles, 1.5);
    let mut bad = Vec::new();
    for &((x, y), c) in &want {
        let gp = g[(y * gw + x) as usize];
        let sp = s[(y * sw + x) as usize];
        if off(gp, c) > TOL || off(sp, c) > TOL {
            bad.push(format!(
                "({x}, {y}): GPU {:?}, software {:?}, Chrome {c:?}",
                &gp[..3],
                &sp[..3]
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// #1755: a red/blue 10px gradient tile animated from `0` to `1px` in 1/16px
/// steps, DPR 1 and 1.5. Each step's row matches the software painter (whose
/// pattern wraps its taps, and whose values `background_subpixel_1151_tests`
/// pins against Chrome) within 2 levels, and the red in one 10px period stays
/// what the software painter's does. Clamped at the seams, the GPU's boundary
/// pixel held until the fraction reached 0.5 and then jumped, the period's
/// red swinging ±9%; placed on whole pixels, it did not move until a pixel's
/// worth of travel.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_sub_pixel_position_animation_moves_smoothly() {
    let mut gpu = Gpu::new();
    let mut bad = Vec::new();
    for scale in [1.0, 1.5] {
        for k in 0..=16 {
            let style = format!("background: {RB} {}px 0 / 10px 10px", k as f64 / 16.0);
            let (gw, g) = gpu.render(&[&style], scale);
            let (sw, s) = software(&[&style], scale);
            let y = (7.0 * scale) as u32;
            let period = (10.0 * scale) as u32;
            let (mut gr, mut sr) = (0u32, 0u32);
            for x in 0..(60.0 * scale) as u32 {
                let gp = g[(y * gw + x) as usize];
                let sp = s[(y * sw + x) as usize];
                if off(gp, [sp[0], sp[1], sp[2]]) > 2 {
                    bad.push(format!(
                        "{scale}x, {k}/16 px, pixel {x}: GPU {:?}, software {:?}",
                        &gp[..3],
                        &sp[..3]
                    ));
                }
                if (period..2 * period).contains(&x) {
                    gr += u32::from(gp[0]);
                    sr += u32::from(sp[0]);
                }
            }
            if gr.abs_diff(sr) > 2 * period {
                bad.push(format!(
                    "{scale}x, {k}/16 px: a period's red is {gr} on the GPU, {sr} in software"
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{} pixels off:\n{bad:#?}", bad.len());
}
