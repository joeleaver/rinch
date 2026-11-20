//! #2740 on the GPU painter: pixels registered with `register_rgba` and
//! shown by `<img src="rinch-image:…">`, painted by `VelloPainter` and
//! rendered by Vello, against the pixels `rinch-dom`'s
//! `decoded_rgba_2740_tests` holds the software painter to. The app's buffer
//! reaches Vello as the same allocation (`DecodedImage::blob`).
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_decoded_rgba_2740 -- --ignored`.
#![cfg(feature = "gpu-renderer")]

use peniko::Brush;
use rinch::image::{register_rgba, unregister_rgba};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::FileImageLoader;
use rinch_dom::paint::vello_painter::VelloPainter;
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 120;
const H: u32 = 60;

/// 4x2, straight alpha: red, green, blue, half-transparent blue / black,
/// white, yellow, magenta.
fn picture() -> Arc<[u8]> {
    [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [0, 0, 255, 128],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
    ]
    .concat()
    .into()
}

fn render(doc: &mut RinchDocument) -> Vec<[u8; 4]> {
    doc.resolve_layout(W as f32, H as f32);
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
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_texture(
            &device,
            &queue,
            painter.scene(),
            &view,
            &vello::RenderParams {
                base_color: peniko::Color::WHITE,
                width: W,
                height: H,
                antialiasing_method: vello::AaConfig::Area,
            },
        )
        .unwrap();
    let row = (W * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
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

/// The centre of each 10x10 block of the 40x20 `<img>` at (10, 10).
const EXPECTED: &[((u32, u32), [u8; 3])] = &[
    ((15, 15), [255, 0, 0]),
    ((25, 15), [0, 255, 0]),
    ((35, 15), [0, 0, 255]),
    ((45, 15), [127, 127, 255]),
    ((15, 25), [0, 0, 0]),
    ((25, 25), [255, 255, 255]),
    ((35, 25), [255, 255, 0]),
    ((45, 25), [255, 0, 255]),
    ((55, 15), [255, 255, 255]),
];

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_registered_pixels_paint_as_given() {
    let src = register_rgba("gpu-2740", 4, 2, picture()).unwrap();
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: #fff; }");
    doc.tree.image_loader = Some(Arc::new(FileImageLoader));
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", &src);
    doc.set_attribute(
        img,
        "style",
        "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px; \
         image-rendering: pixelated; display: block",
    );
    doc.append_child(body, img);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wide = false;
    while doc.tree.image_cache.get(&src).is_none() {
        assert!(Instant::now() < deadline, "the picture never arrived");
        wide = !wide;
        doc.resolve_layout(W as f32 + if wide { 1.0 } else { 0.0 }, H as f32);
        std::thread::sleep(Duration::from_millis(2));
    }
    let px = render(&mut doc);
    for &((x, y), want) in EXPECTED {
        let p = px[(y * W + x) as usize];
        let got = [p[0], p[1], p[2]];
        let close = got
            .iter()
            .zip(want)
            .all(|(&g, w)| (g as i32 - w as i32).abs() <= 1);
        assert!(close, "({x}, {y}) is {got:?}, want {want:?}");
    }
    unregister_rgba("gpu-2740");
}
