//! #2614: the GPU painter draws the hyphen at a soft hyphen a line breaks
//! at, as the software painter does
//! (`rinch-dom/tests/soft_hyphen_2614_tests.rs`,
//! `the_software_painter_draws_the_hyphen`): it is the soft hyphen's own
//! glyph on that line, so both painters draw it from the same glyph runs.
//! Red before #2614: nothing was drawn right of `Extra`.
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_soft_hyphen_2614 -- --ignored`.
#![cfg(feature = "gpu")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 80;
const H: u32 = 50;

const INTER: &[u8] = include_bytes!("../../rinch-dom/assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
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
    doc.load_css(
        "* { margin: 0; padding: 0 } html, body { background: #fff; font: 16px/20px ProbeFace }",
    );
    let body = doc.body();
    doc.set_inner_html(
        body,
        "<div style='width:50px;color:#000'>Extra&shy;ordinary</div>",
    );
    doc.resolve_layout(W as f32, H as f32);
    doc
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

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_the_hyphen_is_drawn() {
    let mut doc = doc();
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let pixels = Gpu::new().render(painter.scene(), vello::AaConfig::Area);
    let darkest = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| pixels[(y * W + x) as usize][0])
            .min()
            .unwrap()
    };
    assert!(darkest(40, 44, 0, 20) < 96, "the hyphen is drawn");
    assert_eq!(darkest(46, 80, 0, 20), 255);
    assert_eq!(darkest(64, 80, 20, 40), 255);
}
